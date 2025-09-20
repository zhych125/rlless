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
use crate::render::ui::ViewState;
use crate::search::SearchOptions;
use std::sync::Arc;
use tokio::sync::mpsc::{Sender, UnboundedReceiver};
use tokio::time::{self, Duration};

/// Tracks render-related state that must persist across input actions and worker responses.
pub struct RenderLoopState {
    search_state: Option<Arc<SearchHighlightSpec>>,
    search_options: SearchOptions,
    pending_options_update: bool,
}

impl RenderLoopState {
    pub fn new(search_options: SearchOptions) -> Self {
        Self {
            search_state: None,
            search_options,
            pending_options_update: false,
        }
    }

    pub fn highlight_spec(&self) -> Option<Arc<SearchHighlightSpec>> {
        self.search_state.clone()
    }

    pub fn search_options(&self) -> &SearchOptions {
        &self.search_options
    }

    pub fn set_search_options(&mut self, options: SearchOptions) {
        self.search_options = options;
        self.refresh_active_search();
    }

    pub fn clear_search(&mut self) {
        self.search_state = None;
        self.pending_options_update = false;
    }

    pub fn set_search(&mut self, search: Arc<SearchHighlightSpec>) {
        self.search_state = Some(search);
        self.pending_options_update = false;
    }

    fn refresh_active_search(&mut self) {
        if let Some(spec) = self.search_state.as_ref() {
            let updated = Arc::new(SearchHighlightSpec {
                pattern: Arc::clone(&spec.pattern),
                options: self.search_options.clone(),
            });
            self.search_state = Some(updated);
        } else {
            self.pending_options_update = true;
        }
    }

    fn search_options_summary(&self) -> String {
        format!(
            "search options: case={} regex={} word={}",
            if self.search_options.case_sensitive {
                "sensitive"
            } else {
                "ignore"
            },
            if self.search_options.regex_mode {
                "on"
            } else {
                "off"
            },
            if self.search_options.whole_word {
                "on"
            } else {
                "off"
            }
        )
    }

    fn ensure_active_search(&self, view_state: &mut ViewState) -> bool {
        if self.search_state.is_some() {
            true
        } else {
            view_state
                .status_line
                .set_message("No active search".to_string());
            false
        }
    }

    async fn queue_viewport_update(
        &self,
        request: ViewportRequest,
        view_state: &mut ViewState,
        search_tx: &mut Sender<SearchCommand>,
        next_request_id: &mut RequestId,
        latest_view_request: &mut Option<RequestId>,
    ) -> Result<bool> {
        view_state.at_eof = false;
        self.request_viewport(
            request,
            view_state,
            search_tx,
            next_request_id,
            latest_view_request,
        )
        .await?;
        Ok(true)
    }

    async fn queue_match_navigation(
        &self,
        traversal: MatchTraversal,
        view_state: &mut ViewState,
        search_tx: &mut Sender<SearchCommand>,
        next_request_id: &mut RequestId,
        latest_search_request: &mut Option<RequestId>,
    ) -> Result<bool> {
        let request_id = *next_request_id;
        *next_request_id += 1;
        *latest_search_request = Some(request_id);
        search_tx
            .send(SearchCommand::NavigateMatch {
                request_id,
                traversal,
                current_top: view_state.viewport_top_byte,
            })
            .await
            .map_err(|_| RllessError::other("search worker unavailable"))?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn process_action(
        &mut self,
        action: InputAction,
        view_state: &mut ViewState,
        search_tx: &mut Sender<SearchCommand>,
        next_request_id: &mut RequestId,
        latest_view_request: &mut Option<RequestId>,
        latest_search_request: &mut Option<RequestId>,
        pending_search_state: &mut Option<(RequestId, Arc<SearchHighlightSpec>)>,
    ) -> Result<bool> {
        match action {
            InputAction::Quit => Ok(false),
            InputAction::Scroll { direction, lines } => {
                let delta = match direction {
                    ScrollDirection::Up => -(lines as i64),
                    ScrollDirection::Down => lines as i64,
                };
                self.queue_viewport_update(
                    ViewportRequest::RelativeLines {
                        anchor: view_state.viewport_top_byte,
                        lines: delta,
                    },
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await
            }
            InputAction::PageUp => {
                self.queue_viewport_update(
                    ViewportRequest::RelativeLines {
                        anchor: view_state.viewport_top_byte,
                        lines: -(view_state.lines_per_page() as i64),
                    },
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await
            }
            InputAction::PageDown => {
                self.queue_viewport_update(
                    ViewportRequest::RelativeLines {
                        anchor: view_state.viewport_top_byte,
                        lines: view_state.lines_per_page() as i64,
                    },
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await
            }
            InputAction::GoToStart => {
                self.queue_viewport_update(
                    ViewportRequest::Absolute(0),
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await
            }
            InputAction::GoToEnd => {
                self.queue_viewport_update(
                    ViewportRequest::EndOfFile,
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await
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
                    return Ok(true);
                }

                let options = self.search_options.clone();
                let pattern: Arc<str> = Arc::from(trimmed.to_string());
                let request_id = *next_request_id;
                *next_request_id += 1;
                *latest_search_request = Some(request_id);
                let highlight = Arc::new(SearchHighlightSpec {
                    pattern: Arc::clone(&pattern),
                    options: options.clone(),
                });
                pending_search_state.replace((request_id, Arc::clone(&highlight)));

                search_tx
                    .send(SearchCommand::ExecuteSearch {
                        request_id,
                        pattern,
                        direction,
                        options,
                        origin_byte: view_state.viewport_top_byte,
                    })
                    .await
                    .map_err(|_| RllessError::other("search worker unavailable"))?;
                Ok(true)
            }
            InputAction::NextMatch => {
                if !self.ensure_active_search(view_state) {
                    if self.pending_options_update {
                        view_state
                            .status_line
                            .set_message("Search options updated; start a new search.".to_string());
                    }
                    return Ok(true);
                }
                self.queue_match_navigation(
                    MatchTraversal::Next,
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_search_request,
                )
                .await
            }
            InputAction::PreviousMatch => {
                if !self.ensure_active_search(view_state) {
                    if self.pending_options_update {
                        view_state
                            .status_line
                            .set_message("Search options updated; start a new search.".to_string());
                    }
                    return Ok(true);
                }
                self.queue_match_navigation(
                    MatchTraversal::Previous,
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_search_request,
                )
                .await
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
            InputAction::StartPercentInput => {
                view_state.status_line.set_message("goto: %".to_string());
                Ok(true)
            }
            InputAction::UpdatePercentBuffer(buffer) => {
                let display = if buffer.is_empty() {
                    "goto: %".to_string()
                } else {
                    format!("goto: %{}", buffer)
                };
                view_state.status_line.set_message(display);
                Ok(true)
            }
            InputAction::CancelPercentInput => {
                view_state.status_line.clear_message();
                Ok(true)
            }
            InputAction::SubmitPercent(percent) => {
                let Some(file_size) = view_state.file_size else {
                    view_state
                        .status_line
                        .set_message("Cannot jump: file size unknown".to_string());
                    return Ok(true);
                };

                if file_size == 0 {
                    view_state
                        .status_line
                        .set_message("Cannot jump: file is empty".to_string());
                    return Ok(true);
                }

                if percent >= 100 {
                    view_state
                        .status_line
                        .set_message("goto: 100% (EOF)".to_string());
                    return self
                        .queue_viewport_update(
                            ViewportRequest::EndOfFile,
                            view_state,
                            search_tx,
                            next_request_id,
                            latest_view_request,
                        )
                        .await;
                }

                let target = ((percent as u128) * (file_size as u128) / 100) as u64;
                view_state
                    .status_line
                    .set_message(format!("goto: {}%", percent));
                self.queue_viewport_update(
                    ViewportRequest::Absolute(target),
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                )
                .await
            }
            InputAction::StartCommand => {
                view_state.status_line.set_message("command: -".to_string());
                Ok(true)
            }
            InputAction::UpdateCommandBuffer(buffer) => {
                view_state.status_line.set_message(if buffer.is_empty() {
                    "command: -".to_string()
                } else {
                    format!("command: -{}", buffer)
                });
                Ok(true)
            }
            InputAction::CancelCommand => {
                view_state.status_line.clear_message();
                Ok(true)
            }
            InputAction::ExecuteCommand { buffer } => {
                if buffer.is_empty() {
                    view_state
                        .status_line
                        .set_message("No command entered".to_string());
                    return Ok(true);
                }

                let mut options_changed = false;
                for flag in buffer.chars() {
                    match flag {
                        'i' | 'I' => {
                            self.search_options.case_sensitive =
                                !self.search_options.case_sensitive;
                            options_changed = true;
                        }
                        'r' | 'R' => {
                            if !self.search_options.regex_mode {
                                self.search_options.regex_mode = true;
                                options_changed = true;
                            }
                        }
                        'n' | 'N' => {
                            if self.search_options.regex_mode {
                                self.search_options.regex_mode = false;
                                options_changed = true;
                            }
                        }
                        'w' | 'W' => {
                            self.search_options.whole_word = !self.search_options.whole_word;
                            options_changed = true;
                        }
                        other => {
                            view_state
                                .status_line
                                .set_message(format!("Unknown command flag: {}", other));
                            return Ok(true);
                        }
                    }
                }

                if options_changed {
                    self.refresh_active_search();
                    view_state
                        .status_line
                        .set_message(self.search_options_summary());
                    self.request_viewport(
                        ViewportRequest::Absolute(view_state.viewport_top_byte),
                        view_state,
                        search_tx,
                        next_request_id,
                        latest_view_request,
                    )
                    .await?;
                } else {
                    view_state
                        .status_line
                        .set_message("Search options unchanged".to_string());
                }

                Ok(true)
            }
            InputAction::NoAction | InputAction::InvalidInput => Ok(true),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn handle_response(
        &mut self,
        response: SearchResponse,
        view_state: &mut ViewState,
        latest_view_request: &mut Option<RequestId>,
        latest_search_request: &mut Option<RequestId>,
        pending_search_state: &mut Option<(RequestId, Arc<SearchHighlightSpec>)>,
        search_tx: &mut Sender<SearchCommand>,
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
        search_tx: &mut Sender<SearchCommand>,
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

/// Orchestrates the main render loop once channels have been wired.
pub struct RenderCoordinator;

impl RenderCoordinator {
    #[allow(clippy::too_many_arguments)]
    async fn process_pending_actions(
        state: &mut RenderLoopState,
        actions: &mut Vec<InputAction>,
        view_state: &mut ViewState,
        search_tx: &mut Sender<SearchCommand>,
        next_request_id: &mut RequestId,
        latest_view_request: &mut Option<RequestId>,
        latest_search_request: &mut Option<RequestId>,
        pending_search_state: &mut Option<(RequestId, Arc<SearchHighlightSpec>)>,
    ) -> Result<bool> {
        for action in actions.drain(..) {
            if !state
                .process_action(
                    action,
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                    latest_search_request,
                    pending_search_state,
                )
                .await?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    async fn drain_search_responses(
        state: &mut RenderLoopState,
        view_state: &mut ViewState,
        search_resp_rx: &mut tokio::sync::mpsc::Receiver<SearchResponse>,
        latest_view_request: &mut Option<RequestId>,
        latest_search_request: &mut Option<RequestId>,
        pending_search_state: &mut Option<(RequestId, Arc<SearchHighlightSpec>)>,
        search_tx: &mut Sender<SearchCommand>,
        next_request_id: &mut RequestId,
    ) -> Result<()> {
        while let Ok(response) = search_resp_rx.try_recv() {
            state
                .handle_response(
                    response,
                    view_state,
                    latest_view_request,
                    latest_search_request,
                    pending_search_state,
                    search_tx,
                    next_request_id,
                )
                .await?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        state: &mut RenderLoopState,
        view_state: &mut ViewState,
        ui_renderer: &mut dyn crate::render::ui::UIRenderer,
        input_rx: &mut UnboundedReceiver<InputAction>,
        search_tx: &mut Sender<SearchCommand>,
        search_resp_rx: &mut tokio::sync::mpsc::Receiver<SearchResponse>,
        next_request_id: &mut RequestId,
        latest_view_request: &mut Option<RequestId>,
        latest_search_request: &mut Option<RequestId>,
        pending_search_state: &mut Option<(RequestId, Arc<SearchHighlightSpec>)>,
    ) -> Result<()> {
        let mut interval = time::interval(Duration::from_millis(16));
        let mut action_buffer = Vec::new();
        let mut running = true;

        while running {
            interval.tick().await;

            while let Ok(action) = input_rx.try_recv() {
                action_buffer.push(action);
            }

            running = running
                && Self::process_pending_actions(
                    state,
                    &mut action_buffer,
                    view_state,
                    search_tx,
                    next_request_id,
                    latest_view_request,
                    latest_search_request,
                    pending_search_state,
                )
                .await?;

            if !running {
                break;
            }

            Self::drain_search_responses(
                state,
                view_state,
                search_resp_rx,
                latest_view_request,
                latest_search_request,
                pending_search_state,
                search_tx,
                next_request_id,
            )
            .await?;

            ui_renderer.render(view_state)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;
    use crate::input::InputStateMachine;
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

    #[test]
    fn percent_jump_requires_digits() {
        let mut sm = InputStateMachine::new();
        assert_eq!(
            sm.handle_key_event(key(KeyCode::Char('%'))),
            InputAction::StartPercentInput
        );

        assert_eq!(
            sm.handle_key_event(key(KeyCode::Char('1'))),
            InputAction::UpdatePercentBuffer("1".to_string())
        );
        assert_eq!(
            sm.handle_key_event(key(KeyCode::Char('0'))),
            InputAction::UpdatePercentBuffer("10".to_string())
        );
        assert_eq!(
            sm.handle_key_event(key(KeyCode::Enter)),
            InputAction::SubmitPercent(10)
        );
    }
}
