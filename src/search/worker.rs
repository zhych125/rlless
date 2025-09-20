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
    mut search_engine: RipgrepEngine,
) {
    let mut context: Option<SearchContext> = None;

    while let Some(cmd) = rx.recv().await {
        match cmd {
            SearchCommand::LoadViewport {
                request_id,
                top,
                page_lines,
                highlights,
            } => {
                let response = load_viewport(
                    request_id,
                    &*file_accessor,
                    &search_engine,
                    top,
                    page_lines,
                    highlights,
                )
                .await;

                match response {
                    Ok(resp) => {
                        if tx.send(resp).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = tx
                            .send(SearchResponse::Error {
                                request_id,
                                error: err,
                            })
                            .await;
                    }
                }
            }
            SearchCommand::ExecuteSearch {
                request_id,
                pattern,
                direction,
                options,
                origin_byte,
            } => {
                let result = execute_search(
                    request_id,
                    &mut search_engine,
                    &mut context,
                    pattern,
                    direction,
                    options,
                    origin_byte,
                )
                .await;

                if tx.send(result).await.is_err() {
                    break;
                }
            }
            SearchCommand::NavigateMatch {
                request_id,
                traversal,
                current_top,
            } => {
                let result = navigate_match(
                    request_id,
                    &file_accessor,
                    &mut search_engine,
                    &mut context,
                    traversal,
                    current_top,
                )
                .await;

                if tx.send(result).await.is_err() {
                    break;
                }
            }
            SearchCommand::UpdateSearchContext(new_context) => {
                context = Some(new_context);
            }
            SearchCommand::Shutdown => break,
        }
    }
}

async fn load_viewport(
    request_id: RequestId,
    file_accessor: &dyn FileAccessor,
    search_engine: &RipgrepEngine,
    top: ViewportRequest,
    page_lines: usize,
    highlights: Option<SearchHighlightSpec>,
) -> Result<SearchResponse> {
    let target_byte = match top {
        ViewportRequest::Absolute(byte) => byte,
        ViewportRequest::RelativeLines { anchor, lines } => {
            if lines == 0 {
                anchor
            } else if lines > 0 {
                file_accessor
                    .next_page_start(anchor, lines as usize)
                    .await?
            } else {
                file_accessor
                    .prev_page_start(anchor, (-lines) as usize)
                    .await?
            }
        }
        ViewportRequest::EndOfFile => file_accessor.last_page_start(page_lines).await?,
    };

    let lines = file_accessor
        .read_from_byte(target_byte, page_lines)
        .await?;
    let highlights_vec = if let Some(spec) = highlights {
        compute_highlights(search_engine, &spec, &lines)?
    } else {
        vec![Vec::new(); lines.len()]
    };

    let file_size = file_accessor.file_size();
    let at_eof = if lines.is_empty() {
        true
    } else {
        let next_start = file_accessor
            .next_page_start(target_byte, page_lines.max(1))
            .await?;
        next_start >= file_size
    };

    Ok(SearchResponse::ViewportLoaded {
        request_id,
        top_byte: target_byte,
        lines,
        highlights: highlights_vec,
        at_eof,
        file_size,
    })
}

fn compute_highlights(
    search_engine: &RipgrepEngine,
    spec: &SearchHighlightSpec,
    lines: &[String],
) -> Result<Vec<Vec<(usize, usize)>>> {
    let mut all_highlights = Vec::with_capacity(lines.len());
    for line in lines {
        let ranges = search_engine.get_line_matches(&spec.pattern, line, &spec.options)?;
        all_highlights.push(ranges);
    }
    Ok(all_highlights)
}

async fn execute_search(
    request_id: RequestId,
    search_engine: &mut RipgrepEngine,
    context: &mut Option<SearchContext>,
    pattern: String,
    direction: SearchDirection,
    options: SearchOptions,
    origin_byte: u64,
) -> SearchResponse {
    let mut new_context = SearchContext {
        pattern: pattern.clone(),
        direction,
        options: options.clone(),
        last_match_byte: None,
    };

    let search_future = match direction {
        SearchDirection::Forward => search_engine.search_from(&pattern, origin_byte, &options),
        SearchDirection::Backward => search_engine.search_prev(&pattern, origin_byte, &options),
    };

    match search_future.await {
        Ok(Some(byte)) => {
            new_context.last_match_byte = Some(byte);
            *context = Some(new_context);
            SearchResponse::SearchCompleted {
                request_id,
                match_byte: Some(byte),
                message: None,
            }
        }
        Ok(None) => {
            *context = Some(new_context);
            SearchResponse::SearchCompleted {
                request_id,
                match_byte: None,
                message: Some("Pattern not found".to_string()),
            }
        }
        Err(err) => SearchResponse::Error {
            request_id,
            error: err,
        },
    }
}

async fn navigate_match(
    request_id: RequestId,
    file_accessor: &Arc<dyn FileAccessor>,
    search_engine: &mut RipgrepEngine,
    context: &mut Option<SearchContext>,
    traversal: MatchTraversal,
    current_top: u64,
) -> SearchResponse {
    let ctx = match context.as_mut() {
        Some(ctx) => ctx,
        None => {
            return SearchResponse::SearchCompleted {
                request_id,
                match_byte: None,
                message: Some("No active search".to_string()),
            };
        }
    };

    let options = ctx.options.clone();
    let pattern = ctx.pattern.clone();

    let start_result = match traversal {
        MatchTraversal::Next => {
            if ctx.direction == SearchDirection::Forward {
                next_line_start(file_accessor, current_top).await
            } else {
                prev_line_start(file_accessor, current_top).await
            }
        }
        MatchTraversal::Previous => {
            if ctx.direction == SearchDirection::Forward {
                prev_line_start(file_accessor, current_top).await
            } else {
                next_line_start(file_accessor, current_top).await
            }
        }
    };

    let start_byte = match start_result {
        Ok(byte) => byte,
        Err(err) => {
            return SearchResponse::Error {
                request_id,
                error: err,
            };
        }
    };

    let search_future = match traversal {
        MatchTraversal::Next => match ctx.direction {
            SearchDirection::Forward => search_engine.search_from(&pattern, start_byte, &options),
            SearchDirection::Backward => search_engine.search_prev(&pattern, start_byte, &options),
        },
        MatchTraversal::Previous => match ctx.direction {
            SearchDirection::Forward => search_engine.search_prev(&pattern, start_byte, &options),
            SearchDirection::Backward => search_engine.search_from(&pattern, start_byte, &options),
        },
    };

    match search_future.await {
        Ok(Some(byte)) => {
            ctx.last_match_byte = Some(byte);
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
        Err(err) => SearchResponse::Error {
            request_id,
            error: err,
        },
    }
}

async fn next_line_start(file_accessor: &Arc<dyn FileAccessor>, current_byte: u64) -> Result<u64> {
    let new_byte = file_accessor.next_page_start(current_byte, 1).await?;
    if new_byte == file_accessor.file_size() {
        Ok(current_byte)
    } else {
        Ok(new_byte)
    }
}

async fn prev_line_start(file_accessor: &Arc<dyn FileAccessor>, current_byte: u64) -> Result<u64> {
    if current_byte == 0 {
        Ok(0)
    } else {
        file_accessor.prev_page_start(current_byte, 1).await
    }
}
