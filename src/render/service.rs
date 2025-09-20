//! Render coordination helpers.
//!
//! Provides the state machine that mediates between input actions, search commands, and view
//! updates. The high-level render loop currently lives in `Application::run`, but will be migrated
//! into this module across subsequent phases.

use crate::error::{Result, RllessError};
use crate::input::{InputAction, ScrollDirection};
use crate::render::protocol::{
    MatchTraversal, RequestId, SearchCommand, SearchHighlightSpec, SearchResponse, ViewportRequest,
};
use crate::search::SearchOptions;
use crate::ui::ViewState;
use tokio::sync::mpsc;

/// Tracks render-related state that must persist across input actions and worker responses.
#[derive(Default)]
pub struct RenderLoopState {
    search_state: Option<SearchHighlightSpec>,
}

impl RenderLoopState {
    pub fn new() -> Self {
        Self { search_state: None }
    }

    pub fn highlight_spec(&self) -> Option<SearchHighlightSpec> {
        self.search_state.as_ref().map(|state| SearchHighlightSpec {
            pattern: state.pattern.clone(),
            options: state.options.clone(),
        })
    }

    pub fn clear_search(&mut self) {
        self.search_state = None;
    }

    pub fn set_search(&mut self, search: SearchHighlightSpec) {
        self.search_state = Some(search);
    }

    pub async fn process_action(
        &mut self,
        action: InputAction,
        view_state: &mut ViewState,
        search_tx: &mut mpsc::Sender<SearchCommand>,
        next_request_id: &mut RequestId,
        latest_view_request: &mut Option<RequestId>,
        latest_search_request: &mut Option<RequestId>,
        pending_search_state: &mut Option<(RequestId, SearchHighlightSpec)>,
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
                self.clear_search();
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
                let highlight = SearchHighlightSpec {
                    pattern: trimmed.to_string(),
                    options: options.clone(),
                };
                pending_search_state.replace((request_id, highlight.clone()));

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

    pub async fn handle_response(
        &mut self,
        response: SearchResponse,
        view_state: &mut ViewState,
        latest_view_request: &mut Option<RequestId>,
        latest_search_request: &mut Option<RequestId>,
        pending_search_state: &mut Option<(RequestId, SearchHighlightSpec)>,
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
                            self.clear_search();
                        }
                    }
                } else if let Some(byte) = match_byte {
                    view_state.status_line.clear_search_prompt();
                    view_state.status_line.message = None;
                    if let Some((pending_id, state)) = pending_search_state.take() {
                        if pending_id == request_id {
                            self.set_search(state);
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

#[cfg(test)]
mod state_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn navigation_scrolls_and_pages() {
        let mut sm = InputStateMachine::new();
        assert_eq!(
            sm.handle_key_event(key(KeyCode::Char('j'))),
            InputAction::Scroll {
                direction: ScrollDirection::Down,
                lines: 1,
            }
        );
        assert_eq!(
            sm.handle_key_event(key(KeyCode::Char('k'))),
            InputAction::Scroll {
                direction: ScrollDirection::Up,
                lines: 1,
            }
        );
    }
}
