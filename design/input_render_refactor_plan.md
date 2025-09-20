# Input/Render/Search Module Refactor

## Goals
- Adopt modern Rust module conventions: prefer flat exports (`mod.rs` only when unavoidable) and keep public APIs surfaced via explicit `pub use` statements in the parent module file.
- Separate responsibilities for raw input collection, input interpretation, rendering coordination, and search execution.
- Make cross-thread communication explicit through shared protocol types.
- Simplify `Application` so it wires high-level services instead of owning monolithic loops.

## Target Module Layout
- `src/input/`
  - `raw.rs` — wraps crossterm polling and yields low-level input events.
  - `service.rs` — houses the input state machine and exposes `poll_action` for the input thread.
  - `mod.rs` removed; parent `src/input.rs` re-exports selected items.
- `src/render/`
  - `service.rs` — main render loop owning `ViewState`, draining `InputAction`s, coordinating protocol messages, and invoking the renderer.
  - `protocol.rs` — shared enums/structs for communication with the search worker (`SearchCommand`, `SearchResponse`, etc.).
  - `ui.rs` — terminal renderer implementation (current `TerminalUI`, themes, layout helpers).
  - `mod.rs` removed; parent `src/render.rs` re-exports the service and UI traits as needed.
- `src/search/`
  - `worker.rs` — async task that processes protocol commands and emits responses.
  - `engine.rs` — existing ripgrep integration (moved from `search.rs` if necessary).
  - `context.rs` — optional helpers for search navigation state.
- `src/app.rs` — slim bootstrap tying together `input::service`, `render::service`, and `search::worker`.
- Existing modules (`error`, `file_handler`, `ui/theme`, etc.) remain, updated to use new paths.

## Data Flow Overview
1. `input::raw` polls crossterm and produces raw events.
2. `input::service` feeds the state machine and emits domain `InputAction`s to the render thread.
3. `render::service` mutates `ViewState`, pushes commands defined in `render::protocol` to the search worker, and triggers `render::ui` drawing.
4. `search::worker` consumes protocol commands, talks to `file_handler` and search engines, and replies with protocol responses.
5. `render::service` applies responses before the next draw.

## Refactoring Phases
1. **Documentation & Staging**
   - Update `AGENTS.md` with the modern module convention and high-level layout summary.
   - Introduce empty scaffolding files (`input.rs`, `render.rs`, etc.) with `todo!()`/`unimplemented!()` placeholders disabled (commented) until logic migrates.

2. **Input Layer Extraction**
   - Move `InputService`, `InputStateMachine`, and hardware scroll coalescing into the new `input/` module (with coalescing anchored in `raw.rs`).
   - Update the input thread (`spawn_input_thread`) and renderer to use the new paths and a single shared service handle.
   - Revise tests to avoid wall-clock sleeps (inject timeout/clock).

3. **Render & Protocol Split**
   - Extract `SearchCommand`, `SearchResponse`, `ViewportRequest`, etc. into `render::protocol`.
   - Move the coordinator logic from `Application::run` into `render::service`, keeping `ViewState` interactions intact.
   - Adjust `TerminalUI` placement under `render::ui` with flat exports.

4. **Search Worker Consolidation**
   - Relocate worker loop helpers (`search_worker_loop`, `load_viewport`, etc.) into `search::worker` and make them consume the new protocol types.
   - Deduplicate search context handling between render and worker layers.

5. **Application Simplification & Cleanup**
   - Reduce `src/app.rs` to wiring: create services, spawn input thread, run render service, manage shutdown.
   - Remove obsolete modules/files (`src/app/messages.rs`, redundant `ui/events/` directory) and update imports.
   - Run `cargo fmt`, `cargo clippy`, and `cargo test` to validate.

6. **Follow-up Enhancements** (optional)
   - Revisit error propagation and logging across threads.
   - Document channel sizes/policies in the design directory.
