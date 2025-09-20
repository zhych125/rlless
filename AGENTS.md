# Repository Guidelines

## Module Conventions
- Prefer modern flat modules; avoid `mod.rs` unless a crate-level quirk requires it.
- Expose public APIs explicitly from the parent file (e.g., `pub use input::service::InputService;`).
- Keep domain directories focused and under ~300 lines per module; split files by responsibility before they balloon.

## Project Structure & Responsibilities
- `src/app.rs`: application bootstrap that wires the input service, render service, and search worker.
- `src/input/`: raw crossterm polling (`raw.rs`), input state machine (`state.rs`), and the orchestrating input service (`service.rs`).
- `src/render/`: render loop (`service.rs`), renderer implementation (`ui.rs`), and cross-thread protocol definitions (`protocol.rs`).
- `src/search/`: search worker (`worker.rs`), search engine integrations (`engine.rs`), and navigation helpers (`context.rs`).
- `src/file_handler/`: memory-mapped file accessors and compression adapters.
- `src/error.rs`: shared error type.
- `src/main.rs`: CLI bootstrap; `src/lib.rs`: public API surface.
- `design/`: architecture notes; `benches/`: Criterion performance experiments; generated artifacts stay in `target/`.

## Build, Test, and Development Commands
- `cargo fmt --all`: Format the codebase before every commit.
- `cargo clippy --all-targets --all-features`: Lint with warnings treated seriously; fix or gate exceptions.
- `cargo test --all --all-features`: Run the suite, including async components via Tokio.
- `cargo run -- <path-to-log>`: Launch the TUI against a large file for manual smoke testing.
- `cargo bench`: Execute Criterion benchmarks; results appear in `target/criterion/`.

## Coding Style & Naming Conventions
Follow idiomatic Rust with `rustfmt` defaults (4-space indent, trailing commas). Prefer expressive module names mirroring directories. Types and traits use `CamelCase`, functions and variables use `snake_case`, and constants remain `SCREAMING_SNAKE_CASE`. Group imports by crate, then alphabetize. Keep modules under ~300 lines; split submodules when logic spans different concerns (e.g., renderer vs. state management).

## Memory Efficiency Rules
- **Return `Cow<str>` instead of `String`** when possible.
- **Let caller decide**: `.as_ref()` for `&str`, `.into_owned()` for `String`.
- **InMemoryFileAccessor**: Use `Cow::Borrowed` for cached lines (zero allocation).
- **Other accessors**: Use `Cow::Owned` when data must be constructed.

## Testing Guidelines
Unit tests live alongside code in `mod tests`. Use `tokio::test` for async paths and `proptest` for fuzzing edge cases on file parsing. Benchmarks in `benches/` rely on Criterion’s async harness—update baselines only when performance materially improves. Target full coverage for file access, search, and UI state transitions before submitting.

## Commit & Pull Request Guidelines
Commits follow an imperative, present-tense style (`Implement terminal resize event handling`). Scope each commit to a single concern; include reproduction steps in the body when fixing bugs. Pull requests should link related design docs or issues, summarize behavioral impact, list test commands executed, and add screenshots or terminal captures when UI output changes. Request review from a maintainer familiar with the touched module.
