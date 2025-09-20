//! Protocol definitions shared between the render coordinator and the search worker.

use crate::error::RllessError;
use crate::input::SearchDirection;
use crate::search::SearchOptions;

/// Identifier attached to cross-thread requests so responses can be correlated.
pub type RequestId = u64;

/// How the viewport worker should interpret a navigation intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportRequest {
    /// Jump to an absolute byte offset (top of viewport aligns to this byte).
    Absolute(u64),
    /// Move relative to the provided anchor by a number of lines (positive = down).
    RelativeLines { anchor: u64, lines: i64 },
    /// Jump to the logical end of the file (last full page when possible).
    EndOfFile,
}

/// Active search context used to compute highlights inside the viewport worker.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHighlightSpec {
    pub pattern: String,
    pub options: SearchOptions,
}

/// Directional traversal for repeating a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchTraversal {
    Next,
    Previous,
}

/// Canonical search state shared with the background worker so it can
/// resume searches without re-sending all parameters every time.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchContext {
    pub pattern: String,
    pub direction: SearchDirection,
    pub options: SearchOptions,
    pub last_match_byte: Option<u64>,
}

/// Commands sent from the render coordinator to the search/paging worker.
#[derive(Debug, Clone, PartialEq)]
pub enum SearchCommand {
    LoadViewport {
        request_id: RequestId,
        top: ViewportRequest,
        page_lines: usize,
        highlights: Option<SearchHighlightSpec>,
    },
    ExecuteSearch {
        request_id: RequestId,
        pattern: String,
        direction: SearchDirection,
        options: SearchOptions,
        origin_byte: u64,
    },
    NavigateMatch {
        request_id: RequestId,
        traversal: MatchTraversal,
        current_top: u64,
    },
    UpdateSearchContext(SearchContext),
    Shutdown,
}

/// Responses emitted by the search/paging worker back to the coordinator.
#[derive(Debug)]
pub enum SearchResponse {
    ViewportLoaded {
        request_id: RequestId,
        top_byte: u64,
        lines: Vec<String>,
        highlights: Vec<Vec<(usize, usize)>>,
        at_eof: bool,
        file_size: u64,
    },
    SearchCompleted {
        request_id: RequestId,
        match_byte: Option<u64>,
        message: Option<String>,
    },
    Error {
        request_id: RequestId,
        error: RllessError,
    },
}
