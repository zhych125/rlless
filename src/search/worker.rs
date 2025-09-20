use crate::error::Result;
use crate::file_handler::FileAccessor;
use crate::input::SearchDirection;
use crate::render::protocol::{
    MatchTraversal, RequestId, SearchCommand, SearchContext, SearchHighlightSpec, SearchResponse,
    ViewportRequest,
};
use crate::search::{RipgrepEngine, SearchEngine, SearchOptions};
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};

/// Run the search/paging worker processing commands from the coordinator.
pub async fn search_worker_loop(
    mut rx: Receiver<SearchCommand>,
    tx: Sender<SearchResponse>,
    file_accessor: Arc<dyn FileAccessor>,
    search_engine: RipgrepEngine,
) {
    let mut state = WorkerState::new(file_accessor, search_engine);

    while let Some(cmd) = rx.recv().await {
        let outcome = state.handle_command(cmd).await;
        if let Some(response) = outcome.response {
            if tx.send(response).await.is_err() {
                break;
            }
        }

        if outcome.done {
            break;
        }
    }
}

struct WorkerState {
    file_accessor: Arc<dyn FileAccessor>,
    search_engine: RipgrepEngine,
    context: Option<SearchContext>,
    last_highlight: Option<Arc<SearchHighlightSpec>>,
    // Cache of `(page_lines, start_byte)` for the last viewport to avoid redundant
    // `last_page_start` computations while the viewport height stays constant.
    last_page_start: Option<(usize, u64)>,
}

impl WorkerState {
    fn new(file_accessor: Arc<dyn FileAccessor>, search_engine: RipgrepEngine) -> Self {
        Self {
            file_accessor,
            search_engine,
            context: None,
            last_highlight: None,
            last_page_start: None,
        }
    }

    async fn handle_command(&mut self, cmd: SearchCommand) -> HandlerOutcome {
        match cmd {
            SearchCommand::LoadViewport {
                request_id,
                top,
                page_lines,
                highlights,
            } => match self
                .load_viewport(request_id, top, page_lines, highlights)
                .await
            {
                Ok(response) => HandlerOutcome::respond(response),
                Err(error) => HandlerOutcome::respond(SearchResponse::Error { request_id, error }),
            },
            SearchCommand::ExecuteSearch {
                request_id,
                pattern,
                direction,
                options,
                origin_byte,
            } => HandlerOutcome::respond(
                self.execute_search(request_id, pattern, direction, options, origin_byte)
                    .await,
            ),
            SearchCommand::NavigateMatch {
                request_id,
                traversal,
                current_top,
            } => HandlerOutcome::respond(
                self.navigate_match(request_id, traversal, current_top)
                    .await,
            ),
            SearchCommand::UpdateSearchContext(new_context) => {
                self.last_highlight = Some(Arc::new(SearchHighlightSpec {
                    pattern: Arc::clone(&new_context.pattern),
                    options: new_context.options.clone(),
                }));
                self.context = Some(new_context);
                HandlerOutcome::continue_without_response()
            }
            SearchCommand::ClearSearchContext => {
                self.context = None;
                self.last_highlight = None;
                HandlerOutcome::continue_without_response()
            }
            SearchCommand::Shutdown => HandlerOutcome::exit(),
        }
    }

    async fn load_viewport(
        &mut self,
        request_id: RequestId,
        top: ViewportRequest,
        page_lines: usize,
        highlights: Option<Arc<SearchHighlightSpec>>,
    ) -> Result<SearchResponse> {
        let target_byte = self.resolve_viewport_target(top, page_lines).await?;
        let lines = self
            .file_accessor
            .read_from_byte(target_byte, page_lines)
            .await?;
        let highlight_spec = if let Some(spec) = highlights {
            self.last_highlight = Some(Arc::clone(&spec));
            Some(spec)
        } else {
            self.last_highlight.clone()
        };

        let highlights = if let Some(spec) = highlight_spec {
            self.compute_highlights(spec.as_ref(), &lines)?
        } else {
            vec![Vec::new(); lines.len()]
        };

        let file_size = self.file_accessor.file_size();
        let at_eof = self
            .detect_eof(target_byte, page_lines, file_size, &lines)
            .await?;

        Ok(SearchResponse::ViewportLoaded {
            request_id,
            top_byte: target_byte,
            lines,
            highlights,
            at_eof,
            file_size,
        })
    }

    async fn execute_search(
        &mut self,
        request_id: RequestId,
        pattern: Arc<str>,
        direction: SearchDirection,
        options: SearchOptions,
        origin_byte: u64,
    ) -> SearchResponse {
        let mut new_context = SearchContext {
            pattern: Arc::clone(&pattern),
            direction,
            options: options.clone(),
            last_match_byte: None,
        };

        let search_future = match direction {
            SearchDirection::Forward => {
                self.search_engine
                    .search_from(pattern.as_ref(), origin_byte, &options)
            }
            SearchDirection::Backward => {
                self.search_engine
                    .search_prev(pattern.as_ref(), origin_byte, &options)
            }
        };

        match search_future.await {
            Ok(Some(byte)) => {
                new_context.last_match_byte = Some(byte);
                self.last_highlight = Some(Arc::new(SearchHighlightSpec {
                    pattern: Arc::clone(&new_context.pattern),
                    options: new_context.options.clone(),
                }));
                self.context = Some(new_context);
                SearchResponse::SearchCompleted {
                    request_id,
                    match_byte: Some(byte),
                    message: None,
                }
            }
            Ok(None) => {
                self.last_highlight = Some(Arc::new(SearchHighlightSpec {
                    pattern: Arc::clone(&new_context.pattern),
                    options: new_context.options.clone(),
                }));
                self.context = Some(new_context);
                SearchResponse::SearchCompleted {
                    request_id,
                    match_byte: None,
                    message: Some("Pattern not found".to_string()),
                }
            }
            Err(error) => SearchResponse::Error { request_id, error },
        }
    }

    async fn navigate_match(
        &mut self,
        request_id: RequestId,
        traversal: MatchTraversal,
        current_top: u64,
    ) -> SearchResponse {
        let ctx_snapshot = match self.context.as_ref() {
            Some(ctx) => (ctx.direction, ctx.options.clone(), Arc::clone(&ctx.pattern)),
            None => {
                return SearchResponse::SearchCompleted {
                    request_id,
                    match_byte: None,
                    message: Some("No active search".to_string()),
                };
            }
        };

        let (direction, options, pattern) = ctx_snapshot;

        let start_byte = match self
            .start_position_for_navigation(traversal, direction, current_top)
            .await
        {
            Ok(byte) => byte,
            Err(error) => {
                return SearchResponse::Error { request_id, error };
            }
        };

        let result = match (traversal, direction) {
            (MatchTraversal::Next, SearchDirection::Forward)
            | (MatchTraversal::Previous, SearchDirection::Backward) => {
                self.search_engine
                    .search_from(pattern.as_ref(), start_byte, &options)
                    .await
            }
            _ => {
                self.search_engine
                    .search_prev(pattern.as_ref(), start_byte, &options)
                    .await
            }
        };

        match result {
            Ok(Some(byte)) => {
                if let Some(ctx) = self.context.as_mut() {
                    ctx.last_match_byte = Some(byte);
                    self.last_highlight = Some(Arc::new(SearchHighlightSpec {
                        pattern: Arc::clone(&ctx.pattern),
                        options: ctx.options.clone(),
                    }));
                }
                SearchResponse::SearchCompleted {
                    request_id,
                    match_byte: Some(byte),
                    message: None,
                }
            }
            Ok(None) => SearchResponse::SearchCompleted {
                request_id,
                match_byte: None,
                message: Some("Pattern not found".to_string()),
            },
            Err(error) => SearchResponse::Error { request_id, error },
        }
    }

    async fn resolve_viewport_target(
        &mut self,
        top: ViewportRequest,
        page_lines: usize,
    ) -> Result<u64> {
        let file_size = self.file_accessor.file_size();

        if file_size == 0 {
            return Ok(0);
        }

        let last_start = self.compute_last_page_start(page_lines, file_size).await?;

        let mut target_byte = match top {
            ViewportRequest::Absolute(byte) => byte,
            ViewportRequest::RelativeLines { anchor, lines } => {
                if lines == 0 {
                    anchor
                } else if lines > 0 {
                    self.file_accessor
                        .next_page_start(anchor, lines as usize)
                        .await?
                } else {
                    self.file_accessor
                        .prev_page_start(anchor, (-lines) as usize)
                        .await?
                }
            }
            ViewportRequest::EndOfFile => last_start.unwrap_or(0),
        };

        if let Some(last) = last_start {
            if target_byte > last {
                target_byte = last;
            }
        }

        Ok(target_byte)
    }

    async fn compute_last_page_start(
        &mut self,
        page_lines: usize,
        file_size: u64,
    ) -> Result<Option<u64>> {
        if file_size == 0 {
            self.last_page_start = None;
            return Ok(None);
        }

        match self.last_page_start {
            Some((cached_lines, pos)) if cached_lines == page_lines => Ok(Some(pos)),
            _ => {
                let last = self.file_accessor.last_page_start(page_lines).await?;
                self.last_page_start = Some((page_lines, last));
                Ok(Some(last))
            }
        }
    }

    fn compute_highlights(
        &self,
        spec: &SearchHighlightSpec,
        lines: &[String],
    ) -> Result<Vec<Vec<(usize, usize)>>> {
        let mut all_highlights = Vec::with_capacity(lines.len());
        for line in lines {
            let ranges = self
                .search_engine
                .get_line_matches(&spec.pattern, line, &spec.options)?;
            all_highlights.push(ranges);
        }
        Ok(all_highlights)
    }

    async fn detect_eof(
        &self,
        top_byte: u64,
        page_lines: usize,
        file_size: u64,
        lines: &[String],
    ) -> Result<bool> {
        if lines.is_empty() {
            return Ok(true);
        }

        let next_start = self
            .file_accessor
            .next_page_start(top_byte, page_lines.max(1))
            .await?;
        Ok(next_start >= file_size)
    }

    async fn start_position_for_navigation(
        &self,
        traversal: MatchTraversal,
        direction: SearchDirection,
        current_top: u64,
    ) -> Result<u64> {
        match (traversal, direction) {
            (MatchTraversal::Next, SearchDirection::Forward)
            | (MatchTraversal::Previous, SearchDirection::Backward) => {
                self.next_line_start(current_top).await
            }
            _ => self.prev_line_start(current_top).await,
        }
    }

    async fn next_line_start(&self, current_byte: u64) -> Result<u64> {
        let new_byte = self.file_accessor.next_page_start(current_byte, 1).await?;
        if new_byte == self.file_accessor.file_size() {
            Ok(current_byte)
        } else {
            Ok(new_byte)
        }
    }

    async fn prev_line_start(&self, current_byte: u64) -> Result<u64> {
        if current_byte == 0 {
            Ok(0)
        } else {
            self.file_accessor.prev_page_start(current_byte, 1).await
        }
    }
}

struct HandlerOutcome {
    response: Option<SearchResponse>,
    done: bool,
}

impl HandlerOutcome {
    fn respond(response: SearchResponse) -> Self {
        Self {
            response: Some(response),
            done: false,
        }
    }

    fn continue_without_response() -> Self {
        Self {
            response: None,
            done: false,
        }
    }

    fn exit() -> Self {
        Self {
            response: None,
            done: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_handler::accessor::FileAccessor;
    use async_trait::async_trait;
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone)]
    struct EmptyAccessor {
        path: PathBuf,
    }

    impl Default for EmptyAccessor {
        fn default() -> Self {
            Self {
                path: PathBuf::from("<empty>"),
            }
        }
    }

    #[async_trait]
    impl FileAccessor for EmptyAccessor {
        async fn read_from_byte(&self, _start_byte: u64, _max_lines: usize) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn find_next_match(
            &self,
            _start_byte: u64,
            _search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
        ) -> Result<Option<u64>> {
            Ok(None)
        }

        async fn find_prev_match(
            &self,
            _start_byte: u64,
            _search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
        ) -> Result<Option<u64>> {
            Ok(None)
        }

        fn file_size(&self) -> u64 {
            0
        }

        fn file_path(&self) -> &Path {
            &self.path
        }

        async fn last_page_start(&self, _max_lines: usize) -> Result<u64> {
            Ok(0)
        }

        async fn next_page_start(&self, _current_byte: u64, _lines_to_skip: usize) -> Result<u64> {
            Ok(0)
        }

        async fn prev_page_start(&self, _current_byte: u64, _lines_to_skip: usize) -> Result<u64> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn empty_files_resolve_to_zero() {
        let accessor: Arc<dyn FileAccessor> = Arc::new(EmptyAccessor::default());
        let engine = RipgrepEngine::new(Arc::clone(&accessor));
        let mut worker = WorkerState::new(accessor, engine);

        for request in [
            ViewportRequest::Absolute(10),
            ViewportRequest::RelativeLines {
                anchor: 25,
                lines: 3,
            },
            ViewportRequest::EndOfFile,
        ] {
            let resolved = worker.resolve_viewport_target(request, 5).await.unwrap();
            assert_eq!(resolved, 0);
        }
    }
}
