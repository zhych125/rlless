use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use rlless::file_handler::accessor::FileAccessor;
use rlless::input::SearchDirection;
use rlless::render::protocol::{
    MatchTraversal, SearchCommand, SearchContext, SearchHighlightSpec, SearchResponse,
    ViewportRequest,
};
use rlless::search::worker::search_worker_loop;
use rlless::search::SearchOptions;

const TIMEOUT_MS: u64 = 200;

async fn next_response(rx: &mut mpsc::Receiver<SearchResponse>) -> SearchResponse {
    timeout(Duration::from_millis(TIMEOUT_MS), rx.recv())
        .await
        .expect("worker response timed out")
        .expect("worker channel closed unexpectedly")
}

async fn spawn_worker(
    contents: &str,
) -> (
    mpsc::Sender<SearchCommand>,
    mpsc::Receiver<SearchResponse>,
    tokio::task::JoinHandle<()>,
) {
    let (cmd_tx, cmd_rx) = mpsc::channel(4);
    let (resp_tx, resp_rx) = mpsc::channel(4);

    let file = tempfile::NamedTempFile::new().expect("create temp file");
    std::fs::write(file.path(), contents).expect("write contents");

    let raw_accessor = rlless::file_handler::FileAccessorFactory::create(file.path())
        .await
        .expect("create accessor");
    let accessor: Arc<dyn FileAccessor> = Arc::new(raw_accessor);
    let engine = rlless::search::RipgrepEngine::new(Arc::clone(&accessor));

    let worker = tokio::spawn(search_worker_loop(cmd_rx, resp_tx, accessor, engine));

    (cmd_tx, resp_rx, worker)
}

#[tokio::test]
async fn load_viewport_returns_expected_page() {
    let (cmd_tx, mut resp_rx, worker) = spawn_worker("first\nsecond\nthird\nfourth\nfifth\n").await;

    cmd_tx
        .send(SearchCommand::LoadViewport {
            request_id: 1,
            top: ViewportRequest::Absolute(0),
            page_lines: 3,
            highlights: None,
        })
        .await
        .unwrap();

    match next_response(&mut resp_rx).await {
        SearchResponse::ViewportLoaded { lines, .. } => {
            assert_eq!(lines, vec!["first", "second", "third"]);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    cmd_tx.send(SearchCommand::Shutdown).await.unwrap();
    worker.await.unwrap();
}

#[tokio::test]
async fn load_viewport_marks_eof_when_past_file_end() {
    let (cmd_tx, mut resp_rx, worker) = spawn_worker("only\nthis\n").await;

    cmd_tx
        .send(SearchCommand::LoadViewport {
            request_id: 42,
            top: ViewportRequest::Absolute(0),
            page_lines: 10,
            highlights: None,
        })
        .await
        .unwrap();

    match next_response(&mut resp_rx).await {
        SearchResponse::ViewportLoaded { lines, at_eof, .. } => {
            assert_eq!(lines, vec!["only", "this"]);
            assert!(
                at_eof,
                "expected EOF flag when requesting beyond file length"
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    cmd_tx.send(SearchCommand::Shutdown).await.unwrap();
    worker.await.unwrap();
}

#[tokio::test]
async fn relative_scroll_stops_at_last_page() {
    let contents = "line1\nline2\nline3\nline4\nline5\n";
    let (cmd_tx, mut resp_rx, worker) = spawn_worker(contents).await;

    cmd_tx
        .send(SearchCommand::LoadViewport {
            request_id: 1,
            top: ViewportRequest::Absolute(0),
            page_lines: 2,
            highlights: None,
        })
        .await
        .unwrap();

    let first_top = match next_response(&mut resp_rx).await {
        SearchResponse::ViewportLoaded { top_byte, .. } => top_byte,
        other => panic!("unexpected response: {other:?}"),
    };

    cmd_tx
        .send(SearchCommand::LoadViewport {
            request_id: 2,
            top: ViewportRequest::RelativeLines {
                anchor: first_top,
                lines: 10,
            },
            page_lines: 2,
            highlights: None,
        })
        .await
        .unwrap();

    let second_top = match next_response(&mut resp_rx).await {
        SearchResponse::ViewportLoaded {
            top_byte, lines, ..
        } => {
            assert_eq!(lines.last().map(String::as_str), Some("line5"));
            top_byte
        }
        other => panic!("unexpected response: {other:?}"),
    };

    cmd_tx
        .send(SearchCommand::LoadViewport {
            request_id: 3,
            top: ViewportRequest::RelativeLines {
                anchor: second_top,
                lines: 1,
            },
            page_lines: 2,
            highlights: None,
        })
        .await
        .unwrap();

    match next_response(&mut resp_rx).await {
        SearchResponse::ViewportLoaded { top_byte, .. } => {
            assert_eq!(top_byte, second_top);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    cmd_tx.send(SearchCommand::Shutdown).await.unwrap();
    worker.await.unwrap();
}

#[tokio::test]
async fn execute_search_followed_by_viewport_load() {
    let contents = "alpha\nbeta\ngamma\nbeta again\n";
    let (cmd_tx, mut resp_rx, worker) = spawn_worker(contents).await;

    cmd_tx
        .send(SearchCommand::ExecuteSearch {
            request_id: 1,
            pattern: Arc::from("beta"),
            direction: SearchDirection::Forward,
            options: SearchOptions::default(),
            origin_byte: 0,
        })
        .await
        .unwrap();

    let match_byte = match next_response(&mut resp_rx).await {
        SearchResponse::SearchCompleted {
            match_byte: Some(byte),
            message: None,
            ..
        } => byte,
        other => panic!("unexpected response: {other:?}"),
    };

    cmd_tx
        .send(SearchCommand::LoadViewport {
            request_id: 2,
            top: ViewportRequest::Absolute(match_byte),
            page_lines: 2,
            highlights: Some(Arc::new(SearchHighlightSpec {
                pattern: Arc::from("beta"),
                options: SearchOptions::default(),
            })),
        })
        .await
        .unwrap();

    match next_response(&mut resp_rx).await {
        SearchResponse::ViewportLoaded { lines, .. } => {
            assert!(lines.iter().any(|line| line.contains("beta")));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    cmd_tx.send(SearchCommand::Shutdown).await.unwrap();
    worker.await.unwrap();
}

#[tokio::test]
async fn navigate_match_advances_active_context() {
    let contents = "alpha\nbeta\nalpha again\nbeta again\n";
    let (cmd_tx, mut resp_rx, worker) = spawn_worker(contents).await;

    cmd_tx
        .send(SearchCommand::ExecuteSearch {
            request_id: 1,
            pattern: Arc::from("alpha"),
            direction: SearchDirection::Forward,
            options: SearchOptions::default(),
            origin_byte: 0,
        })
        .await
        .unwrap();

    let first_match = match next_response(&mut resp_rx).await {
        SearchResponse::SearchCompleted {
            match_byte: Some(byte),
            ..
        } => byte,
        other => panic!("unexpected response: {other:?}"),
    };

    cmd_tx
        .send(SearchCommand::NavigateMatch {
            request_id: 2,
            traversal: MatchTraversal::Next,
            current_top: first_match,
        })
        .await
        .unwrap();

    let second_match = match next_response(&mut resp_rx).await {
        SearchResponse::SearchCompleted {
            match_byte: Some(byte),
            ..
        } => byte,
        other => panic!("unexpected response: {other:?}"),
    };

    assert!(second_match > first_match);

    cmd_tx.send(SearchCommand::Shutdown).await.unwrap();
    worker.await.unwrap();
}

#[tokio::test]
async fn update_context_enables_navigation_without_execute() {
    let contents = "one\ntwo\nthree\n";
    let (cmd_tx, mut resp_rx, worker) = spawn_worker(contents).await;

    cmd_tx
        .send(SearchCommand::UpdateSearchContext(SearchContext {
            pattern: Arc::from("two"),
            direction: SearchDirection::Forward,
            options: SearchOptions::default(),
            last_match_byte: None,
        }))
        .await
        .unwrap();

    cmd_tx
        .send(SearchCommand::NavigateMatch {
            request_id: 1,
            traversal: MatchTraversal::Next,
            current_top: 0,
        })
        .await
        .unwrap();

    match next_response(&mut resp_rx).await {
        SearchResponse::SearchCompleted {
            match_byte: Some(byte),
            message: None,
            ..
        } => {
            assert!(byte > 0);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    cmd_tx.send(SearchCommand::Shutdown).await.unwrap();
    worker.await.unwrap();
}

#[tokio::test]
async fn execute_search_with_invalid_regex_returns_error() {
    let contents = "abc\n";
    let (cmd_tx, mut resp_rx, worker) = spawn_worker(contents).await;

    let options = SearchOptions {
        regex_mode: true,
        ..SearchOptions::default()
    };

    cmd_tx
        .send(SearchCommand::ExecuteSearch {
            request_id: 7,
            pattern: Arc::from("("),
            direction: SearchDirection::Forward,
            options,
            origin_byte: 0,
        })
        .await
        .unwrap();

    match next_response(&mut resp_rx).await {
        SearchResponse::Error { request_id, .. } => {
            assert_eq!(request_id, 7);
        }
        other => panic!("expected error response, got {other:?}"),
    }

    cmd_tx.send(SearchCommand::Shutdown).await.unwrap();
    worker.await.unwrap();
}
