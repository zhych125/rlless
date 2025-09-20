//! Application orchestration layer
//!
//! Coordinates file access, search, and UI rendering. The new architecture
//! delegates input handling and heavy data operations to background tasks while
//! keeping rendering single-threaded.

use crate::error::{Result, RllessError};
use crate::file_handler::{FileAccessor, FileAccessorFactory};
use crate::input::spawn_input_thread;
use crate::input::InputAction;
use crate::render::protocol::SearchHighlightSpec;
use crate::render::protocol::{RequestId, SearchCommand, SearchResponse, ViewportRequest};
use crate::render::service::{RenderCoordinator, RenderLoopState};
use crate::render::ui::{UIRenderer, ViewState};
use crate::search::worker::search_worker_loop;
use crate::search::RipgrepEngine;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Application orchestrator - coordinates components without duplicating their state
pub struct Application {
    file_accessor: Arc<dyn FileAccessor>,
    ui_renderer: Box<dyn UIRenderer>,
    render_state: RenderLoopState,
}

impl Application {
    /// Create application by initializing and wiring components together
    pub async fn new(file_path: &Path, ui_renderer: Box<dyn UIRenderer>) -> Result<Self> {
        let file_accessor: Arc<dyn FileAccessor> =
            Arc::new(FileAccessorFactory::create(file_path).await?);
        Ok(Self {
            file_accessor,
            ui_renderer,
            render_state: RenderLoopState::new(),
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
        let mut pending_search_state: Option<(RequestId, Arc<SearchHighlightSpec>)> = None;

        // Prime the viewport with initial content
        let initial_req = next_request_id;
        next_request_id += 1;
        latest_view_request = Some(initial_req);
        search_tx
            .send(SearchCommand::LoadViewport {
                request_id: initial_req,
                top: ViewportRequest::Absolute(0),
                page_lines: view_state.lines_per_page() as usize,
                highlights: self.render_state.highlight_spec(),
            })
            .await
            .map_err(|_| RllessError::other("search worker unavailable"))?;

        if let Some(response) = search_resp_rx.recv().await {
            self.render_state
                .handle_response(
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

        RenderCoordinator::run(
            &mut self.render_state,
            &mut view_state,
            self.ui_renderer.as_mut(),
            &mut input_rx,
            &mut search_tx,
            &mut search_resp_rx,
            &mut next_request_id,
            &mut latest_view_request,
            &mut latest_search_request,
            &mut pending_search_state,
        )
        .await?;

        // Graceful shutdown
        shutdown_flag.store(true, Ordering::SeqCst);
        let _ = search_tx.send(SearchCommand::Shutdown).await;
        search_handle.await.ok();
        let _ = input_thread.join();

        self.ui_renderer.cleanup()?;
        Ok(())
    }
}
