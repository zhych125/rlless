# Stage 4 Plan: Search & Navigation Enhancements

## Milestones
1. **Regex-first Search Interface**
   - Make regular-expression search the default behaviour (case-sensitive, literal flags opt-in).
   - Expose CLI flags to opt into literal/word/ignore-case modes.
   - Allow in-app toggles (e.g. `/\c`, `/\R`) to update search options without restarting.
2. **Jump to Percentage Navigation**
   - Support `NUM%` navigation from the input thread and render pipeline.
3. **Search History**
   - Persist search patterns during a session and expose history recall similar to `less` (repeat previous pattern, cycle history on `/` prompt).

## Architecture Considerations
- The render coordinator already owns `RenderLoopState`, responsible for search context and view state. Enhancements will extend that state with search options and history tracking.
- Input subsystem produces `InputAction`s. We will add new actions for runtime flag toggles, search history navigation, and percentage jumps.
- Search worker persists `SearchContext`. We need to expand protocol definitions to include updated options when toggled.

## Tasks by Milestone

### Milestone 1: Regex-first Search Interface
1. **Audit defaults** (render/service + search worker): ensure `SearchOptions::default()` enables regex mode, case-sensitive, word-match off.
2. **Update CLI** (`main.rs`, clap definitions): add flags `--ignore-case/-i`, `--smart-case/-I`, `--word/-w`, `--literal/-n`. Map these into initial `SearchOptions` seeded in `Application::new`.
3. **Runtime toggles**
   - Add a command mode (`-` prefix) that mirrors the search prompt: typing emits `UpdateCommandBuffer`, `Enter` emits `ExecuteCommand` carrying the typed flags, and `Esc` cancels.
   - Update `RenderLoopState` to parse these commands, mutate `SearchOptions`, and reissue viewport/highlight requests.
   - Update the status line to show the in-progress command buffer and to summarize active flags after execution.
4. **Worker support**
   - Ensure `SearchCommand::ExecuteSearch` converts options correctly when toggled mid-session.
   - Integration tests covering CLI + runtime toggles.

### Milestone 2: Jump to Percentage Navigation
1. **Input parsing**: introduce a `%`-triggered "goto" input mode that mirrors command/search prompts (enter to confirm, esc/backspace to edit) and emits structured actions for start/update/submit.
2. **Render handling**: implement coordinate logic in `RenderLoopState::process_action` to convert percentage to byte position (requires file size; fallback to current state if unknown).
3. **Search worker**: already able to load absolute viewports; ensure the jump handles rounding and clamps to file bounds.
4. **Tests**: unit tests for input state machine, integration test to verify `LoadViewport` request hits expected byte.

### Milestone 3: Search History
1. **State storage**: add `VecDeque<String>` (bounded) within `RenderLoopState` to track recent patterns. Push on successful `ExecuteSearch`.
2. **Input enhancements**: support repeat commands and history navigation (e.g., invoking `/` with empty pattern repeats last, adding `InputAction::RecallSearch { offset }`). Map to typical `less` behaviour (`/` recall last pattern, `n`/`N` while buffer empty).
3. **UI feedback**: update status line to display recalled patterns when cycling.
4. **Worker sync**: ensure context updates gracefully when history recall reuses previous patterns.
5. **Testing**: unit tests for history API, integration test verifying repeated search uses stored pattern.

## Additional Considerations
- Extend design doc once UX decisions (key bindings, CLI names) are finalized.
- Document toggle commands and search flags in `AGENTS.md` or user-facing README.
- Evaluate persistence beyond session (out of scope for this stage but note for future).
