# Search Enhancements Stage 5 Design

## Overview
This stage pursues two improvements that lift responsiveness under heavy searches and shrink startup costs on large archives.

1. Allow users to cancel in-flight searches (especially long-running regex queries) via `Ctrl+C` without destabilising the UI threads.
2. Add incremental decompression support for codecs that can surface byte slices without full extraction, reducing file-open latency for large compressed inputs.

## Goals
- Translate `Ctrl+C` into a cross-thread cancel signal that safely unwinds the async search worker while keeping input/render loops responsive.
- Surface cancellation feedback to the render loop so users see that the search was aborted.
- Introduce chunked decompression paths for seekable codecs (e.g., zstd-seekable, BGZF) and integrate them with the adaptive accessor logic.
- Maintain compatibility with existing code paths: fall back gracefully when a codec lacks partial-decompress capabilities.

## Non-Goals
- No redesign of the input state machine beyond the new cancel action.
- Search algorithms remain unchanged aside from adding cooperative cancel points.
- Cross-platform SIGINT plumbing is out-of-scope; we focus on in-app cancellation signals.
- We will not add new codecs in this stage; only leverage capabilities detected today.

## Search Cancellation

### Pain Points
- `Ctrl+C` currently backs users out of command mode but does not abort work already delegated to the search worker thread.
- Long-running regex queries can stall perceived UI responsiveness since the worker keeps running even when the user tries to escape.

### Requirements
- Dispatch `Ctrl+C` from the input service as a `SearchAction::Cancel` whenever the search worker is executing.
- Ensure the search worker broadcasts a `SearchStatus::Cancelled` message to the render loop so the UI can clear progress indicators.
- Guard against races: the worker must exit promptly and release resources, leaving the search state machine ready for the next query.

### Proposed Changes
- **Input Layer (`src/input/service.rs`):**
  - Map `Ctrl+C` to a new `InputEvent::CancelSearch` when a search is active.
  - Emit a `SearchAction::Cancel` through the existing command channel.
- **Shared Cancellation Token:**
  - Store a `CancellationToken` (Tokio’s or a lightweight custom type) within the search context.
  - On each search request, clone the token; on cancel, call `cancel()` and refresh the token for the next run.
- **Search Worker (`src/search/worker.rs`):**
  - Wrap long-running loops in `tokio::select!` against the cancellation token.
  - Insert cooperative checks in regex iteration, literal scanning, and context prefetch loops.
  - Emit `SearchProtocol::Cancelled` before returning so downstream listeners can update state.
- **Render Loop (`src/render/service.rs`, `src/render/protocol.rs`):**
  - Handle the cancelled message by clearing “searching…” affordances and pushing a transient toast/state change.
  - Reset any progress bars or highlight state to the last committed result.

### Testing & Validation
- Unit tests for input service ensuring `Ctrl+C` yields cancel actions in search mode.
- Async integration test simulating a slow search future and asserting early exit after cancellation.
- Manual smoke: run `cargo run` on a large file, trigger a heavy regex, cancel midway.

### Open Questions
- Should `Ctrl+C` cancel only search or also exit command mode when idle? (Default: search cancel when worker busy; otherwise preserve current behaviour.)
- How to debounce repeated cancel signals if the worker is already unwinding?

## Incremental Decompression

### Motivation
Full-file decompression for large archives causes noticeable startup delays. Some codecs (BGZF, zstd-seekable) support retrieving chunks by byte range, which we can exploit for lazy loading and scrolling.

### Requirements
- Detect codecs capable of random-access slices and expose that through the accessor trait.
- Keep a bounded in-memory cache of recently decompressed sections, respecting existing memory constraints.
- Ensure non-seekable codecs continue to work unchanged.

### Proposed Changes
- **Accessor Capability Flag:**
  - Extend `FileAccessor` with `fn supports_segment_reads(&self) -> bool` and `fn read_segment(&self, range: Range<usize>) -> Result<Cow<'_, [u8]>>`.
  - Implement these methods in incremental-capable adapters; others return `false` or delegate to full decompress.
- **Factory Integration (`src/file_handler/factory.rs`):**
  - Detect support during accessor construction (e.g., zstd with seekable frame index).
  - Wrap incremental accessors in a `ChunkCache` that manages LRU eviction and chunk-size policy (configurable via CLI/env).
- **Adaptive Accessor (`src/file_handler/adaptive`):**
  - Update line-fetch logic to request only needed byte ranges when the underlying accessor supports it.
  - Maintain page-level metadata so navigation can prefetch adjacent chunks without blocking.
- **Threading Considerations:**
  - Decompression remains on the file-handler thread; we avoid new threads but ensure operations are cancel-aware via shared token if shared with search.

### Testing & Benchmarking
- Extend `file_opening` Criterion benches to cover incremental vs. full decompression paths.
- Unit tests verifying cache hits/misses, eviction, and fallbacks when segment reads are unsupported.
- Manual smoke: open large gzip archive, jump to distant offsets, confirm reduced wait time.

### Open Questions
- Codec priority order (zstd-seekable vs. bgzf) and availability of indices at runtime.
- Cache sizing policy (fixed chunk count vs. memory budget with telemetry?).
- Configuration toggle: CLI flag, config file, or environment variable?

## Milestones
1. Ship search cancellation end-to-end with tests and UI feedback.
2. Land accessor trait extensions and capability detection for incremental decompression.
3. Add chunk cache integration within the adaptive accessor and ensure search respects cache boundaries.
4. Update benchmarks and capture before/after metrics for both search and file-opening suites.

