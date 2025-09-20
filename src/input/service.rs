//! High-level input service that owns the state machine and raw input collector.
//!
//! This module glues the low-level crossterm polling from [`crate::input::raw`] with the
//! navigation/search state machine from [`crate::input::state`], producing domain
//! [`InputAction`] values for the rest of the application.

use crate::error::Result;
use crate::input::raw::{InputCoalescer, RawInputCollector, RawInputEvent, ScrollDirection};
use crate::input::state::{InputAction, InputStateMachine};
use ratatui::crossterm::event::Event;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Service responsible for producing high-level `InputAction`s from terminal events.
pub struct InputService {
    state_machine: InputStateMachine,
    raw_input: RawInputCollector,
    scroll_coalescer: InputCoalescer,
    pending_actions: VecDeque<InputAction>,
}

impl InputService {
    /// Create a new input service with default coalescing settings.
    pub fn new() -> Self {
        Self {
            state_machine: InputStateMachine::new(),
            raw_input: RawInputCollector::new(),
            scroll_coalescer: InputCoalescer::with_default_window(),
            pending_actions: VecDeque::new(),
        }
    }

    /// Create an input service with a custom coalescing window (useful for tests).
    pub fn with_coalesce_window(window: Duration) -> Self {
        Self {
            state_machine: InputStateMachine::new(),
            raw_input: RawInputCollector::with_window(window),
            scroll_coalescer: InputCoalescer::new(window),
            pending_actions: VecDeque::new(),
        }
    }

    /// Retrieve the next high-level input action.
    ///
    /// This mirrors the legacy behaviour so existing callers can keep a poll-based model while we
    /// migrate to the new threaded architecture.
    pub fn poll_action(&mut self, timeout: Option<Duration>) -> Result<Option<InputAction>> {
        if let Some(action) = self.pending_actions.pop_front() {
            return Ok(Some(action));
        }

        if let Some(action) = self.flush_scroll_if_stale() {
            return Ok(Some(action));
        }

        if let Some(raw_event) = self.raw_input.poll_event(timeout)? {
            self.process_raw_event(raw_event);
            return Ok(self.pending_actions.pop_front());
        }

        if let Some(action) = self.flush_scroll_if_stale() {
            return Ok(Some(action));
        }

        Ok(None)
    }

    /// Process a synthetic crossterm event (primarily used by unit tests).
    pub fn process_event(&mut self, event: Event) {
        self.raw_input.process_event(event);
        while let Some(raw_event) = self.raw_input.pop_pending() {
            self.process_raw_event(raw_event);
        }
    }

    /// Flush accumulated scrolls when the coalescing window has expired.
    pub fn try_flush_coalesced(&mut self) -> Option<InputAction> {
        if let Some(raw_event) = self.raw_input.try_flush_coalesced() {
            self.process_raw_event(raw_event);
        }

        if let Some(action) = self.flush_scroll_if_stale() {
            return Some(action);
        }

        self.pending_actions.pop_front()
    }

    /// Check whether the service has no pending actions or scroll accumulation.
    pub fn is_idle(&self) -> bool {
        self.pending_actions.is_empty()
            && self.raw_input.is_idle()
            && self.scroll_coalescer.is_empty()
    }

    fn process_raw_event(&mut self, event: RawInputEvent) {
        match event {
            RawInputEvent::Key(key_event) => {
                let action = self.state_machine.handle_key_event(key_event);
                self.queue_action(action);
            }
            RawInputEvent::Resize { width, height } => {
                self.queue_action(InputAction::Resize { width, height });
            }
            RawInputEvent::Scroll { direction, lines } => {
                let action = match direction {
                    ScrollDirection::Up => InputAction::ScrollUp(lines),
                    ScrollDirection::Down => InputAction::ScrollDown(lines),
                };
                self.queue_action(action);
            }
        }
    }

    fn queue_action(&mut self, action: InputAction) {
        match action {
            InputAction::ScrollUp(lines) => {
                self.queue_scroll(ScrollDirection::Up, lines);
            }
            InputAction::ScrollDown(lines) => {
                self.queue_scroll(ScrollDirection::Down, lines);
            }
            InputAction::NoAction | InputAction::InvalidInput => {}
            _ => {
                self.flush_scroll();
                self.pending_actions.push_back(action);
            }
        }
    }

    fn queue_scroll(&mut self, direction: ScrollDirection, lines: u64) {
        let now = Instant::now();
        if let Some((dir, total_lines)) = self.scroll_coalescer.push(direction, lines, now) {
            self.pending_actions
                .push_back(Self::scroll_action(dir, total_lines));
        }
    }

    fn flush_scroll(&mut self) {
        if let Some((dir, lines)) = self.scroll_coalescer.flush() {
            self.pending_actions
                .push_back(Self::scroll_action(dir, lines));
        }
    }

    fn flush_scroll_if_stale(&mut self) -> Option<InputAction> {
        self.scroll_coalescer
            .flush_if_stale(Instant::now())
            .map(|(dir, lines)| Self::scroll_action(dir, lines))
    }

    fn scroll_action(direction: ScrollDirection, lines: u64) -> InputAction {
        match direction {
            ScrollDirection::Up => InputAction::ScrollUp(lines),
            ScrollDirection::Down => InputAction::ScrollDown(lines),
        }
    }
}

impl Default for InputService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::state::SearchDirection;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key_press(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn coalesces_repeated_scroll_down_keys() {
        let mut service = InputService::with_coalesce_window(Duration::from_millis(5));

        service.process_event(key_press(KeyCode::Char('j')));
        service.process_event(key_press(KeyCode::Char('j')));
        service.process_event(key_press(KeyCode::Char('j')));

        assert!(service.try_flush_coalesced().is_none());

        std::thread::sleep(Duration::from_millis(6));
        let action = service.try_flush_coalesced().unwrap();
        assert_eq!(action, InputAction::ScrollDown(3));
    }

    #[test]
    fn flushes_on_direction_change() {
        let mut service = InputService::with_coalesce_window(Duration::from_millis(10));

        service.process_event(key_press(KeyCode::Char('j')));
        service.process_event(key_press(KeyCode::Char('j')));
        service.process_event(key_press(KeyCode::Char('k')));

        let first = service.try_flush_coalesced().unwrap();
        assert_eq!(first, InputAction::ScrollDown(2));

        assert!(service.try_flush_coalesced().is_none());
        std::thread::sleep(Duration::from_millis(12));
        let second = service.try_flush_coalesced().unwrap();
        assert_eq!(second, InputAction::ScrollUp(1));
    }

    #[test]
    fn flushes_before_non_scroll_action() {
        let mut service = InputService::with_coalesce_window(Duration::from_millis(10));

        service.process_event(key_press(KeyCode::Char('j')));
        service.process_event(key_press(KeyCode::Char('j')));
        service.process_event(key_press(KeyCode::Char('g')));

        let first = service.try_flush_coalesced().unwrap();
        assert_eq!(first, InputAction::ScrollDown(2));

        let second = service.try_flush_coalesced().unwrap();
        assert_eq!(second, InputAction::GoToStart);
    }

    #[test]
    fn translates_resize_events() {
        let mut service = InputService::new();
        service.process_event(Event::Resize(120, 40));

        let action = service.try_flush_coalesced().unwrap();
        assert_eq!(
            action,
            InputAction::Resize {
                width: 120,
                height: 40
            }
        );
    }

    #[test]
    fn forwards_search_actions() {
        let mut service = InputService::new();
        service.process_event(key_press(KeyCode::Char('/')));
        service.process_event(key_press(KeyCode::Char('t')));
        service.process_event(key_press(KeyCode::Char('e')));
        service.process_event(key_press(KeyCode::Char('s')));
        service.process_event(key_press(KeyCode::Char('t')));
        service.process_event(key_press(KeyCode::Enter));

        let start = service.try_flush_coalesced().unwrap();
        assert_eq!(start, InputAction::StartSearch(SearchDirection::Forward));

        // Flush buffered updates (test, for simplicity)
        let mut collected = Vec::new();
        while let Some(action) = service.try_flush_coalesced() {
            collected.push(action);
        }

        assert!(collected.contains(&InputAction::ExecuteSearch {
            pattern: "test".to_string(),
            direction: SearchDirection::Forward,
        }));
    }
}
