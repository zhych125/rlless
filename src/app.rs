//! Application orchestration layer
//!
//! Coordinates file access, search, and UI rendering. The new architecture
//! delegates input handling and heavy data operations to background tasks while
//! keeping rendering single-threaded.

pub mod messages;
mod runtime;

use crate::app::messages::{
    MatchTraversal, RequestId, SearchCommand, SearchHighlightSpec, SearchResponse, ViewportRequest,
};
use crate::app::runtime::{search_worker_loop, spawn_input_thread};
use crate::error::{Result, RllessError};
use crate::file_handler::{FileAccessor, FileAccessorFactory};
use crate::input::{InputAction, ScrollDirection};
use crate::search::{RipgrepEngine, SearchOptions};
use crate::ui::{UIRenderer, ViewState};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time;

/// Application orchestrator - coordinates components without duplicating their state
pub struct Application {
    file_accessor: Arc<dyn FileAccessor>,
    ui_renderer: Box<dyn UIRenderer>,
    /// Current search results and navigation state
    search_state: Option<SearchState>,
}

/// Minimal search state for less-like navigation
#[derive(Clone)]
struct SearchState {
    pattern: String,
    options: SearchOptions,
}

impl Application {
    /// Create application by initializing and wiring components together
    pub async fn new(file_path: &Path, ui_renderer: Box<dyn UIRenderer>) -> Result<Self> {
        let file_accessor: Arc<dyn FileAccessor> =
            Arc::new(FileAccessorFactory::create(file_path).await?);
        Ok(Self {
            file_accessor,
            ui_renderer,
            search_state: None,
        })
    }

    /// Run the application using the multi-threaded input/search architecture
    pub async fn run(&mut self) -> Result<()> {
        self.ui_renderer.initialize()?;

        let (width, height) = self.ui_renderer.get_terminal_size()?;
        let file_path = self.file_accessor.file_path().to_path_buf();
        let mut view_state = ViewState::new(file_path, width, height);

        let (input_tx, mut input_rx) = mpsc::unbounded_channel::<InputAction>();
        let (mut search_tx, search_rx) = mpsc::channel::<SearchCommand>(64);
        let (search_resp_tx, mut search_resp_rx) = mpsc::channel::<SearchResponse>(64);

        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let input_thread =
            spawn_input_thread(input_tx, shutdown_flag.clone(), Duration::from_millis(12));

        let worker_accessor = Arc::clone(&self.file_accessor);
        let worker_engine = RipgrepEngine::new(Arc::clone(&self.file_accessor));
        let search_handle = tokio::spawn(search_worker_loop(
            search_rx,
            search_resp_tx,
            worker_accessor,
            worker_engine,
        ));

        let mut next_request_id: RequestId = 1;
        #[allow(unused_assignments)]
        let mut latest_view_request: Option<RequestId> = None;
        let mut latest_search_request: Option<RequestId> = None;
        let mut pending_search_state: Option<(RequestId, SearchState)> = None;

        // Prime the viewport with initial content
        let initial_req = next_request_id;
        next_request_id += 1;
        latest_view_request = Some(initial_req);
        search_tx
            .send(SearchCommand::LoadViewport {
                request_id: initial_req,
                top: ViewportRequest::Absolute(0),
                page_lines: view_state.lines_per_page() as usize,
                highlights: self.highlight_spec(),
            })
            .await
            .map_err(|_| RllessError::other("search worker unavailable"))?;

        if let Some(response) = search_resp_rx.recv().await {
            self.handle_response(
                response,
                &mut view_state,
                &mut latest_view_request,
                &mut latest_search_request,
                &mut pending_search_state,
                &mut search_tx,
                &mut next_request_id,
            )
            .await?;
        }

        let mut interval = time::interval(Duration::from_millis(16));
        let mut action_buffer = Vec::new();
        let mut running = true;

        while running {
            interval.tick().await;

            while let Ok(action) = input_rx.try_recv() {
                action_buffer.push(action);
            }

            for action in action_buffer.drain(..) {
                if !self
                    .process_action(
                        action,
                        &mut view_state,
                        &mut search_tx,
                        &mut next_request_id,
                        &mut latest_view_request,
                        &mut latest_search_request,
                        &mut pending_search_state,
                    )
                    .await?
                {
                    running = false;
                    break;
                }
            }

            while let Ok(response) = search_resp_rx.try_recv() {
                self.handle_response(
                    response,
                    &mut view_state,
                    &mut latest_view_request,
                    &mut latest_search_request,
                    &mut pending_search_state,
                    &mut search_tx,
                    &mut next_request_id,
                )
                .await?;
            }

            self.ui_renderer.render(&view_state)?;
        }

        // Graceful shutdown
        shutdown_flag.store(true, Ordering::SeqCst);
        let _ = search_tx.send(SearchCommand::Shutdown).await;
        search_handle.await.ok();
        let _ = input_thread.join();

        self.ui_renderer.cleanup()?;
        Ok(())
    }

    fn highlight_spec(&self) -> Option<SearchHighlightSpec> {
        self.search_state.as_ref().map(|state| SearchHighlightSpec {
            pattern: state.pattern.clone(),
            options: state.options.clone(),
        })
    }

    async fn process_action(
        &mut self,
        action: InputAction,
        view_state: &mut ViewState,
        search_tx: &mut mpsc::Sender<SearchCommand>,
        next_request_id: &mut RequestId,
        latest_view_request: &mut Option<RequestId>,
        latest_search_request: &mut Option<RequestId>,
        pending_search_state: &mut Option<(RequestId, SearchState)>,
    ) -> Result<bool> {
        match action {
            InputAction::Quit => Ok(false),
            InputAction::Scroll { direction, lines } => {
                view_state.at_eof = false;
                let delta = match direction {
                    ScrollDirection::Up => -(lines as i64),
                    ScrollDirection::Down => lines as i64,
                };
                self.request_viewport(
                    ViewportRequest::RelativeLines {
                        anchor: view_state.viewport_top_byte,
                        lines: delta,
                    },
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await?;
                Ok(true)
            }
            InputAction::PageUp => {
                view_state.at_eof = false;
                self.request_viewport(
                    ViewportRequest::RelativeLines {
                        anchor: view_state.viewport_top_byte,
                        lines: -(view_state.lines_per_page() as i64),
                    },
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await?;
                Ok(true)
            }
            InputAction::PageDown => {
                view_state.at_eof = false;
                self.request_viewport(
                    ViewportRequest::RelativeLines {
                        anchor: view_state.viewport_top_byte,
                        lines: view_state.lines_per_page() as i64,
                    },
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await?;
                Ok(true)
            }
            InputAction::GoToStart => {
                view_state.at_eof = false;
                self.request_viewport(
                    ViewportRequest::Absolute(0),
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await?;
                Ok(true)
            }
            InputAction::GoToEnd => {
                view_state.at_eof = false;
                self.request_viewport(
                    ViewportRequest::EndOfFile,
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await?;
                Ok(true)
            }
            InputAction::StartSearch(direction) => {
                view_state.status_line.set_search_prompt(direction);
                Ok(true)
            }
            InputAction::UpdateSearchBuffer { direction, buffer } => {
                view_state
                    .status_line
                    .update_search_prompt(direction, buffer);
                Ok(true)
            }
            InputAction::CancelSearch => {
                view_state.status_line.clear_search_prompt();
                view_state.status_line.message = None;
                self.search_state = None;
                pending_search_state.take();
                self.request_viewport(
                    ViewportRequest::Absolute(view_state.viewport_top_byte),
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await?;
                Ok(true)
            }
            InputAction::ExecuteSearch { pattern, direction } => {
                let trimmed = pattern.trim();
                if trimmed.is_empty() {
                    view_state.status_line.clear_search_prompt();
                    view_state.status_line.message = None;
                    return Ok(true);
                }

                let options = SearchOptions::default();
                let request_id = *next_request_id;
                *next_request_id += 1;
                *latest_search_request = Some(request_id);
                pending_search_state.replace((
                    request_id,
                    SearchState {
                        pattern: trimmed.to_string(),
                        options: options.clone(),
                    },
                ));

                search_tx
                    .send(SearchCommand::ExecuteSearch {
                        request_id,
                        pattern: trimmed.to_string(),
                        direction,
                        options,
                        origin_byte: view_state.viewport_top_byte,
                    })
                    .await
                    .map_err(|_| RllessError::other("search worker unavailable"))?;
                Ok(true)
            }
            InputAction::NextMatch => {
                if self.search_state.is_none() {
                    view_state
                        .status_line
                        .set_message("No active search".to_string());
                    return Ok(true);
                }
                let request_id = *next_request_id;
                *next_request_id += 1;
                *latest_search_request = Some(request_id);
                search_tx
                    .send(SearchCommand::NavigateMatch {
                        request_id,
                        traversal: MatchTraversal::Next,
                        current_top: view_state.viewport_top_byte,
                    })
                    .await
                    .map_err(|_| RllessError::other("search worker unavailable"))?;
                Ok(true)
            }
            InputAction::PreviousMatch => {
                if self.search_state.is_none() {
                    view_state
                        .status_line
                        .set_message("No active search".to_string());
                    return Ok(true);
                }
                let request_id = *next_request_id;
                *next_request_id += 1;
                *latest_search_request = Some(request_id);
                search_tx
                    .send(SearchCommand::NavigateMatch {
                        request_id,
                        traversal: MatchTraversal::Previous,
                        current_top: view_state.viewport_top_byte,
                    })
                    .await
                    .map_err(|_| RllessError::other("search worker unavailable"))?;
                Ok(true)
            }
            InputAction::Resize { width, height } => {
                if view_state.update_terminal_size(width, height) {
                    self.request_viewport(
                        ViewportRequest::Absolute(view_state.viewport_top_byte),
                        view_state,
                        search_tx,
                        next_request_id,
                        latest_view_request,
                    )
                    .await?;
                }
                Ok(true)
            }
            InputAction::NoAction | InputAction::InvalidInput => Ok(true),
        }
    }

    async fn handle_response(
        &mut self,
        response: SearchResponse,
        view_state: &mut ViewState,
        latest_view_request: &mut Option<RequestId>,
        latest_search_request: &mut Option<RequestId>,
        pending_search_state: &mut Option<(RequestId, SearchState)>,
        search_tx: &mut mpsc::Sender<SearchCommand>,
        next_request_id: &mut RequestId,
    ) -> Result<()> {
        match response {
            SearchResponse::ViewportLoaded {
                request_id,
                top_byte,
                lines,
                highlights,
                at_eof,
                file_size,
            } => {
                if Some(request_id) != *latest_view_request {
                    return Ok(());
                }
                *latest_view_request = None;
                view_state.navigate_to_byte(top_byte);
                view_state.at_eof = at_eof;
                view_state.update_viewport_content(lines, highlights);
                view_state.file_size = Some(file_size);
            }
            SearchResponse::SearchCompleted {
                request_id,
                match_byte,
                message,
            } => {
                if Some(request_id) != *latest_search_request {
                    return Ok(());
                }
                *latest_search_request = None;

                if let Some(msg) = message {
                    view_state.status_line.clear_search_prompt();
                    view_state.status_line.set_message(msg);
                    if let Some((pending_id, _)) = pending_search_state {
                        if *pending_id == request_id {
                            pending_search_state.take();
                            self.search_state = None;
                        }
                    }
                } else if let Some(byte) = match_byte {
                    view_state.status_line.clear_search_prompt();
                    view_state.status_line.message = None;
                    if let Some((pending_id, state)) = pending_search_state.take() {
                        if pending_id == request_id {
                            self.search_state = Some(state);
                        }
                    }
                    view_state.at_eof = false;
                    let request_id = self
                        .request_viewport(
                            ViewportRequest::Absolute(byte),
                            view_state,
                            search_tx,
                            next_request_id,
                            latest_view_request,
                        )
                        .await?;
                    *latest_view_request = Some(request_id);
                }
            }
            SearchResponse::Error { request_id, error } => {
                if Some(request_id) == *latest_view_request {
                    *latest_view_request = None;
                }
                if Some(request_id) == *latest_search_request {
                    *latest_search_request = None;
                    pending_search_state.take();
                }
                view_state
                    .status_line
                    .set_message(format!("Operation failed: {}", error));
            }
        }
        Ok(())
    }

    async fn request_viewport(
        &self,
        top: ViewportRequest,
        view_state: &ViewState,
        search_tx: &mut mpsc::Sender<SearchCommand>,
        next_request_id: &mut RequestId,
        latest_view_request: &mut Option<RequestId>,
    ) -> Result<RequestId> {
        let request_id = *next_request_id;
        *next_request_id += 1;
        let _ = latest_view_request.replace(request_id);
        search_tx
            .send(SearchCommand::LoadViewport {
                request_id,
                top,
                page_lines: view_state.lines_per_page() as usize,
                highlights: self.highlight_spec(),
            })
            .await
            .map_err(|_| RllessError::other("search worker unavailable"))?;
        Ok(request_id)
    }
}
