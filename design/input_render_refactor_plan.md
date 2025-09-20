# Input/Render/Search Module Refactor

## Goals
- Adopt modern Rust module conventions: prefer flat exports (`mod.rs` only when unavoidable) and keep public APIs surfaced via explicit `pub use` statements in the parent module file.
- Separate responsibilities for raw input collection, input interpretation, rendering coordination, and search execution.
- Make cross-thread communication explicit through shared protocol types.
- Simplify `Application` so it wires high-level services instead of owning monolithic loops.

## Target Module Layout
- `src/input/`
  - `raw.rs` — wraps crossterm polling and yields low-level input events.
  - `service.rs` — houses the input state machine and exposes the input thread helper.
  - `src/input.rs` re-exports the public surface; no `mod.rs` files.
- `src/render/`
  - `service.rs` — render coordinator that drains actions, talks to the search worker, and triggers drawing.
  - `protocol.rs` — shared enums/structs for communication with the search worker (`SearchCommand`, `SearchResponse`, etc.).
  - `ui/` — terminal renderer implementation (renderer trait, view state, terminal backend, theme helpers).
- `src/search/`
  - `worker.rs` — async task that processes protocol commands and emits responses.
  - `core.rs` — ripgrep integration (`SearchEngine`, `SearchOptions`).
- `src/app.rs` — slim bootstrap tying together input, render, and search subsystems.
- Existing modules (`error`, `file_handler`, etc.) remain, updated to use the new paths.

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
   - ✅ `InputService`, `InputStateMachine`, and scroll coalescing now live under `src/input/` with the thread helper exposed at `input::spawn_input_thread`.
   - ⚠️ Tests still rely on short `std::thread::sleep` windows to flush the coalescer; reevaluate if deterministic clock injection becomes necessary.

3. **Render & Protocol Split**
   - ✅ Protocol types live in `render::protocol` and the coordinator is in `render::service`.
   - ✅ `render::ui` hosts renderer traits, terminal backend, state, and theme modules using flat exports.

4. **Search Worker Consolidation**
   - ✅ `search::worker` owns the command loop and helper methods in a dedicated `WorkerState`.
   - ✅ Render/worker share `SearchContext` via protocol updates instead of ad-hoc duplication.
   - ✅ Integration tests (`tests/search_worker.rs`) exercise load, search, navigation, context updates, EOF, and error paths over real channels.

5. **Application Simplification & Cleanup**
   - ✅ `Application::run` now wires services, spawns the input/search tasks, and defers loops to `render::service`.
   - ✅ Legacy facades (`src/ui.rs`, `src/app/runtime.rs`) and directory placeholders removed.
   - ✅ Continuous formatting/linting/testing ensures the refactor stays in sync.

6. **Follow-up Enhancements** (optional)
   - Revisit cross-thread logging/error reporting (current worker logs to stderr on fatal error).
   - Document channel sizing policies and any back-pressure expectations once stabilized.
