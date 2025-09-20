# Threaded Input/Search/Render Refactor Plan

## Goals & Constraints
- Decouple input polling, viewport/search computation, and rendering so each runs on a dedicated thread/task.
- Coalesce high-frequency scroll events before they reach the renderer to avoid redundant work.
- Maintain rendering cadence near 60 Hz while ensuring UI updates remain responsive to search results and resize events.
- Preserve existing abstractions where useful (`FileAccessor`, `RipgrepEngine`, `ViewState`) and keep UI state mutations single-threaded for simplicity.
- Avoid blocking the Tokio runtime; favour `tokio::sync` channels and `spawn_blocking`/`std::thread` only when required by crossterm.

## Alignment With `less` Core Behaviours
- Maintain the canonical navigation semantics (`j`/`k`, arrow keys, `space`, `b`, `g`, `G`) including EOF messaging and percentage display (`src/app.rs:102`–`197`).
- Preserve forward/backward search flow initiated by `/` and `?`, along with `n`/`N` traversal that respects the original search direction (`src/app.rs:198`–`239`).
- Keep the single-line command/status area that shows prompts, messages, and percent-of-file indicators consistent with `less` (`src/ui/state.rs:54`–`114`).
- Ensure resize handling immediately reflows the viewport so behaviour mirrors `less`'s dynamic window adjustment (`src/app.rs:226`).
- Avoid introducing asynchronous UI side-effects that would reorder visible outcomes versus user inputs; inputs must appear to execute in the same order users expect from `less`.
- Treat scroll coalescing as an implementation detail—batched actions must yield the same net position as discrete key presses so muscle memory continues to apply.

## Current Single-Threaded Flow
- `Application::run` drives input handling, search, and rendering on the same async loop (`src/app.rs:46`). Each iteration polls input with a 50 ms timeout, executes the action (which may hit the file accessor and search engine), renders, then sleeps for 10 ms (`src/app.rs:65`–`84`).
- `TerminalUI::handle_input` both polls crossterm and interprets events via the state machine, returning individual `InputAction`s without coalescing (`src/ui/terminal.rs:213`–`240`).
- Search and paging logic live inside `Application::execute_action`, interleaving synchronous file IO and async search calls with UI state updates (`src/app.rs:99`–`240`).

This architecture is simple but constrains responsiveness: every scroll/search blocks rendering until the work completes, and repeated scroll events cannot be smoothed.

## Target Architecture Overview
```
            ┌──────────────────────┐
            │  crossterm event API │
            └──────────┬──────────┘
                       │
                Input Thread
                    │  (coalesced InputAction)
                    ▼
            ┌──────────────────────┐
            │  Render Coordinator  │ 60 Hz tick
            │  (ViewState owner)   │
            └──────────┬──────────┘
                       │ SearchCommand
                       ▼
               Search/Paging Worker
                       │ SearchResponse
                       ▼
            ┌──────────────────────┐
            │   Terminal Renderer  │
            └──────────────────────┘
```

### Thread / Task Responsibilities
- **Input thread**
  - Runs blocking crossterm polling (needs a dedicated OS thread via `std::thread::spawn` or `tokio::task::spawn_blocking`).
- Owns `InputStateMachine` to translate raw events into logical `InputAction`s (`src/input/state.rs`).
  - Coalesces scroll-heavy actions before pushing them to the queue shared with the render coordinator.
  - Pushes resize/search text updates immediately (no coalescing) so the renderer can react within the next tick.

- **Render coordinator (main async task)**
  - Owns `ViewState` and the concrete `TerminalUI` renderer, ensuring drawing happens on a single thread.
  - Ticks at ~16 ms intervals using `tokio::time::interval`. Each tick drains pending coalesced input actions, batches them, and determines resulting commands for the search worker.
  - Handles UI-only actions directly (e.g., changing status messages) and sends work that requires data access to the search worker.
  - Drains search responses before each draw so the rendered frame reflects the latest data.

- **Search/paging worker task**
  - Runs on the Tokio runtime but off the render thread. Owns `FileAccessor` and `RipgrepEngine` instances so file IO and pattern matching happen away from the UI.
  - Processes `SearchCommand`s sequentially to simplify state transitions (preserves order guarantees for repeated commands).
  - Sends `SearchResponse`s (viewport data, search results, errors) back to the coordinator via an `mpsc` channel.

### Channel Topology
- `input_tx/input_rx`: `tokio::sync::mpsc::Sender<InputAction>`; input thread sends, render coordinator receives.
- `search_tx/search_rx`: `tokio::sync::mpsc::Sender<SearchCommand>`; render coordinator sends commands to the worker.
- `search_result_tx/search_result_rx`: `tokio::sync::mpsc::Sender<SearchResponse>`; worker publishes results, coordinator consumes.
- Use bounded channels (e.g., size 64) to protect against runaway producers while keeping latency low. Apply backpressure with `try_send` fallbacks for scroll coalescing.

## Action & Message Shapes
Define new internal enums to formalize cross-thread communication:
```rust
/// Sent from render coordinator → search worker
pub enum SearchCommand {
    LoadViewport {
        request_id: u64,
        top: ViewportRequest,
        page_lines: usize,
        highlights: Option<SearchHighlightSpec>,
    },
    ExecuteSearch {
        request_id: u64,
        pattern: String,
        direction: SearchDirection,
        options: SearchOptions,
        origin_byte: u64,
    },
    NavigateMatch {
        request_id: u64,
        direction: MatchTraversal,
    },
    UpdateSearchContext(SearchContext),
    Shutdown,
}

pub enum SearchResponse {
    ViewportLoaded {
        request_id: u64,
        top_byte: u64,
        lines: Vec<String>,
        highlights: Vec<Vec<(usize, usize)>>,
        at_eof: bool,
        file_size: u64,
    },
    SearchCompleted {
        request_id: u64,
        match_byte: Option<u64>,
        message: Option<String>,
    },
    Error {
        request_id: u64,
        error: RllessError,
    },
}
```
- `ViewportRequest` encapsulates scroll intent (`Absolute(u64)` / `RelativeLines{i64}` / `EndOfFile`).
- `SearchHighlightSpec` bundles the active pattern/options so the worker can reuse compiled matchers without extra chatter.
- `request_id` lets the coordinator correlate responses to user actions; reuse a monotonic counter.

## Scroll Coalescing Strategy
- Replace the current per-event throttling in `TerminalUI::handle_input` (`src/ui/terminal.rs:144`–`185`). Extract the mouse scroll helper into a standalone `InputCoalescer` owned by the input thread.
- Algorithm:
  1. When a scroll `InputAction::ScrollUp(k)`/`ScrollDown(k)` arrives, start or extend an accumulation window.
  2. Keep a running total for each direction while events arrive within a short horizon (e.g., 8–12 ms).
  3. Flush the accumulated scroll when:
     - The direction changes.
     - A non-scroll action is received.
     - The accumulation window expires (use `Instant::now()` comparisons in the input thread).
  4. Emit combined actions using the same enum (e.g., `ScrollDown(total_lines)`), preserving semantics for the coordinator.
- Mouse wheel and repeated key presses both feed the same coalescer. Search-related inputs bypass coalescing entirely to avoid stale prompts.

## Render Loop @ 60 Hz
Pseudo-flow for the coordinator task:
```rust
let mut interval = tokio::time::interval(Duration::from_millis(16));
loop {
    interval.tick().await;

    // 1. Drain input actions
    let mut actions = Vec::new();
    while let Ok(action) = input_rx.try_recv() {
        actions.push(action);
    }

    // 2. Translate actions into state mutations & search commands
    for action in actions {
        match action {
            InputAction::Resize { width, height } => update_viewport_size(...),
            InputAction::Quit => break main_loop,
            scroll_or_search => enqueue_search_command(scroll_or_search, ...),
        }
    }

    // 3. Drain search responses
    while let Ok(response) = search_result_rx.try_recv() {
        apply_search_response(response, &mut view_state);
    }

    // 4. Draw frame
    ui_renderer.render(&view_state)?;
}
```
- Keep UI mutations (status line, prompts, EOF flags) inside the coordinator so rendering remains single-threaded.
- When the loop exits, send `SearchCommand::Shutdown` and join both worker handles before calling `ui_renderer.cleanup()`.

## Search Worker Behaviour
- Spawn as an async task holding `FileAccessor` and `RipgrepEngine`. Use `tokio::select!` to read from `search_rx`, exit on `Shutdown`.
- For `LoadViewport` commands:
  - Resolve target byte using existing helpers (`prev_page_start`, `next_page_start`, `last_page_start`) on the accessor.
  - Read lines via `read_from_byte`, compute highlights when `SearchHighlightSpec` present using `RipgrepEngine::get_line_matches`, and respond with `ViewportLoaded`.
- For search commands:
  - Reuse existing logic from `Application::execute_action` (sections handling `ExecuteSearch`, `NextMatch`, `PreviousMatch` in `src/app.rs:198`–`239`).
  - Maintain an internal `SearchContext` (pattern, direction, options, last_match_byte) to serve `NavigateMatch` requests without extra data in each command.
  - Emit `SearchCompleted` with either a match byte (to drive subsequent viewport load) or a user-facing message (e.g., "Pattern not found").
- Ensure responses carry everything needed so the coordinator does not have to re-query the file accessor.

## UI State Ownership
- `ViewState` remains exclusive to the render coordinator, avoiding shared mutability. Incoming `ViewportLoaded` replaces `visible_lines`, `search_highlights`, `viewport_top_byte`, and `file_size` fields directly.
- Status line updates triggered by search results still happen inside the coordinator; responses include contextual messages to set/clear prompts.
- Resizes trigger a `LoadViewport` with fresh dimensions so the worker returns the correct page size.

## Error Handling & Shutdown
- Input thread: on channel send failure, interpret as application shutdown and break the loop.
- Render coordinator: errors from the renderer or search worker responses surface as user-visible status messages and log entries; fatal errors (e.g., channel closed unexpectedly) result in graceful teardown.
- Provide a `CancellationToken` (from `tokio-util`) or simple atomic flag shared between tasks to coordinate shutdown along with `SearchCommand::Shutdown` and dropping senders.

## Implementation Steps
1. **Introduce channel-friendly message types** in a new module (e.g., `src/app/messages.rs`) and wire them into `App`.
2. **Refactor `TerminalUI`** to split rendering from input: move the state machine and mouse logic into the shared `input::service::InputService` so the renderer no longer owns `InputStateMachine`.
3. **Build the input thread**
   - Create `InputCoalescer` utility.
   - Spawn blocking task during `Application::run` startup that initializes raw mode, consumes crossterm events, coalesces scrolls, and pushes `InputAction`s onto `input_tx`.
4. **Create the search worker task** that owns `FileAccessor`/`RipgrepEngine`, processes `SearchCommand`s, and sends `SearchResponse`s.
5. **Rewrite `Application::run`** into the 60 Hz render coordinator loop: set up channels, spawn tasks, manage `ViewState`, and replace direct calls to `execute_action`/`update_view_content` with command dispatch + response handling.
6. **Port existing action logic** from `execute_action` into coordinator (UI decisions) and worker (IO/search work). Delete or heavily simplify the old `execute_action` method once behaviour parity is confirmed.
7. **Handle shutdown paths** (quit input, drop senders, wait for threads) and ensure `TerminalUI::cleanup` runs even on early errors.
8. **Update tests**
   - Unit-test `InputCoalescer` edge cases.
   - Add integration tests for the worker (driving commands via channels).
   - Adapt UI tests/mocks as needed to accommodate the new renderer structure.
9. **Document the new architecture** (README/design) and adjust any existing design docs referencing the old single-thread loop.

## Task Breakdown (TODOs)
- [ ] Define cross-thread enums (`SearchCommand`, `SearchResponse`, `ViewportRequest`, `SearchHighlightSpec`) and centralize shared types under `src/app/messages.rs`.
- [ ] Extract input handling into `input::service::InputService`, moving `InputStateMachine` ownership out of `TerminalUI` and introducing reusable mouse helpers.
- [ ] Implement `InputCoalescer` utility with unit tests covering direction changes, idle flush, and mixed mouse/keyboard scrolls.
- [ ] Spawn dedicated input thread: initialize raw mode, drive crossterm event loop, apply coalescing, and forward actions via `input_tx`.
- [ ] Stand up render coordinator skeleton inside `Application::run` (interval ticker, action draining, response handling, frame rendering).
- [ ] Build search worker task: command dispatcher, viewport loader, search execution, response emission, and graceful shutdown handling.
- [ ] Migrate existing scroll/search logic from `execute_action` into the appropriate coordinator/worker paths while keeping status updates aligned with `less` behaviour.
- [ ] Wire shutdown signals and ensure dropping channels cleans up the input thread, search worker, and renderer consistently.
- [ ] Expand automated coverage for new modules (coalescer tests, worker channel tests, coordinator tick smoke test) and refresh documentation (README + design directory references).

## Testing & Validation
- Extend existing async/unit tests to cover new message types and worker response logic.
- Add a smoke test that spins up the coordinator with mocked channels to validate 60 Hz tick scheduling behaves (can use a faster interval in tests).
- Run the full suite (`cargo fmt --all`, `cargo clippy --all-targets --all-features`, `cargo test --all --all-features`).
- Manual verification: run `cargo run -- large_test_file.log`, confirm smooth scrolling under rapid wheel/keyboard input and that searches complete without stalling rendering.

## Open Questions / Follow-Ups
- Consider rate-limiting search commands so rapid scrolls do not flood the worker—may require dropping intermediate `LoadViewport` requests when a newer one supersedes them.
- Decide whether to migrate to `crossterm::event::EventStream` (async) after stabilizing the threaded approach; the current plan keeps blocking APIs but isolates them in one thread.
- Investigate whether additional worker threads (e.g., background prefetching) are beneficial once this architecture lands.
