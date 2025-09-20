//! High-level input service.
//!
//! Consumes coalesced raw events, runs the `less`-style input state machine, and yields
//! domain-level `InputAction`s that the render coordinator consumes.

use crate::error::Result;
use crate::input::raw::{RawInputCollector, RawInputEvent};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// Current input mode (`less` navigation vs search prompt).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputState {
    Navigation,
    SearchInput { direction: SearchDirection },
}

/// Direction for forward/backward search.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchDirection {
    Forward,
    Backward,
}

impl SearchDirection {
    /// Character displayed in the search prompt.
    pub fn to_char(self) -> char {
        match self {
            SearchDirection::Forward => '/',
            SearchDirection::Backward => '?',
        }
    }
}

/// Direction for scroll actions emitted by the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
}

/// High-level input actions emitted by the state machine/service.
#[derive(Debug, Clone, PartialEq)]
pub enum InputAction {
    Scroll {
        direction: ScrollDirection,
        lines: u64,
    },
    PageUp,
    PageDown,
    GoToStart,
    GoToEnd,
    Quit,
    StartSearch(SearchDirection),
    UpdateSearchBuffer {
        direction: SearchDirection,
        buffer: String,
    },
    CancelSearch,
    ExecuteSearch {
        pattern: String,
        direction: SearchDirection,
    },
    NextMatch,
    PreviousMatch,
    Resize {
        width: u16,
        height: u16,
    },
    NoAction,
    InvalidInput,
}

/// State machine that mirrors classic `less` bindings.
pub struct InputStateMachine {
    state: InputState,
    search_buffer: String,
}

impl InputStateMachine {
    pub fn new() -> Self {
        Self {
            state: InputState::Navigation,
            search_buffer: String::new(),
        }
    }

    pub fn handle_key_event(&mut self, key_event: KeyEvent) -> InputAction {
        if key_event.kind != KeyEventKind::Press {
            return InputAction::NoAction;
        }

        match (self.state, key_event.code, key_event.modifiers) {
            (InputState::Navigation, KeyCode::Char('j'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                InputAction::Scroll {
                    direction: ScrollDirection::Down,
                    lines: 1,
                }
            }
            (InputState::Navigation, KeyCode::Down, _) => InputAction::Scroll {
                direction: ScrollDirection::Down,
                lines: 1,
            },
            (InputState::Navigation, KeyCode::Char('k'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                InputAction::Scroll {
                    direction: ScrollDirection::Up,
                    lines: 1,
                }
            }
            (InputState::Navigation, KeyCode::Up, _) => InputAction::Scroll {
                direction: ScrollDirection::Up,
                lines: 1,
            },
            (InputState::Navigation, KeyCode::Char(' '), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                InputAction::PageDown
            }
            (InputState::Navigation, KeyCode::Char('f'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                InputAction::PageDown
            }
            (InputState::Navigation, KeyCode::PageDown, _) => InputAction::PageDown,
            (InputState::Navigation, KeyCode::Char('b'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                InputAction::PageUp
            }
            (InputState::Navigation, KeyCode::PageUp, _) => InputAction::PageUp,
            (InputState::Navigation, KeyCode::Char('g'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                InputAction::GoToStart
            }
            (InputState::Navigation, KeyCode::Char('G'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                InputAction::GoToEnd
            }
            (InputState::Navigation, KeyCode::Char('q'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                InputAction::Quit
            }
            (InputState::Navigation, KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                InputAction::Quit
            }
            (InputState::Navigation, KeyCode::Char('n'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                InputAction::NextMatch
            }
            (InputState::Navigation, KeyCode::Char('N'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                InputAction::PreviousMatch
            }
            (InputState::Navigation, KeyCode::Char('/'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.state = InputState::SearchInput {
                    direction: SearchDirection::Forward,
                };
                self.search_buffer.clear();
                InputAction::StartSearch(SearchDirection::Forward)
            }
            (InputState::Navigation, KeyCode::Char('?'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.state = InputState::SearchInput {
                    direction: SearchDirection::Backward,
                };
                self.search_buffer.clear();
                InputAction::StartSearch(SearchDirection::Backward)
            }
            (InputState::SearchInput { .. }, KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.state = InputState::Navigation;
                self.search_buffer.clear();
                InputAction::CancelSearch
            }
            (InputState::SearchInput { direction }, KeyCode::Char(ch), modifiers)
                if (ch.is_ascii_graphic() || ch == ' ')
                    && !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.search_buffer.push(ch);
                InputAction::UpdateSearchBuffer {
                    direction,
                    buffer: self.search_buffer.clone(),
                }
            }
            (InputState::SearchInput { direction }, KeyCode::Backspace, _) => {
                self.search_buffer.pop();
                if self.search_buffer.is_empty() {
                    self.state = InputState::Navigation;
                    InputAction::CancelSearch
                } else {
                    InputAction::UpdateSearchBuffer {
                        direction,
                        buffer: self.search_buffer.clone(),
                    }
                }
            }
            (InputState::SearchInput { direction }, KeyCode::Enter, _) => {
                let pattern = self.search_buffer.clone();
                self.state = InputState::Navigation;
                self.search_buffer.clear();

                if pattern.trim().is_empty() {
                    InputAction::CancelSearch
                } else {
                    InputAction::ExecuteSearch { pattern, direction }
                }
            }
            (InputState::SearchInput { .. }, KeyCode::Esc, _) => {
                self.state = InputState::Navigation;
                self.search_buffer.clear();
                InputAction::CancelSearch
            }
            _ => InputAction::InvalidInput,
        }
    }

    pub fn get_search_buffer(&self) -> &str {
        &self.search_buffer
    }

    pub fn get_state(&self) -> InputState {
        self.state
    }
}

impl Default for InputStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

/// Service responsible for producing high-level `InputAction`s from terminal events.
pub struct InputService {
    state_machine: InputStateMachine,
    raw_input: RawInputCollector,
}

impl InputService {
    pub fn new() -> Self {
        Self {
            state_machine: InputStateMachine::new(),
            raw_input: RawInputCollector::new(),
        }
    }

    pub fn poll_actions(&mut self, timeout: Option<Duration>) -> Result<Vec<InputAction>> {
        let mut actions = Vec::new();

        if let Some(raw_event) = self.raw_input.poll_event(timeout)? {
            if let Some(action) = self.process_raw_event(raw_event) {
                actions.push(action);
            }

            while let Some(extra_event) = self.raw_input.try_flush() {
                if let Some(action) = self.process_raw_event(extra_event) {
                    actions.push(action);
                }
            }
        }

        Ok(actions)
    }

    pub fn process_event(&mut self, event: Event) -> Vec<InputAction> {
        let mut actions = Vec::new();
        self.raw_input.process_event(event);
        while let Some(raw_event) = self.raw_input.try_flush() {
            if let Some(action) = self.process_raw_event(raw_event) {
                actions.push(action);
            }
        }
        actions
    }

    fn process_raw_event(&mut self, event: RawInputEvent) -> Option<InputAction> {
        let action = match event {
            RawInputEvent::Key(key_event) => self.state_machine.handle_key_event(key_event),
            RawInputEvent::Resize { width, height } => InputAction::Resize { width, height },
            RawInputEvent::Scroll { direction, lines } => InputAction::Scroll { direction, lines },
        };

        match action {
            InputAction::NoAction | InputAction::InvalidInput => None,
            _ => Some(action),
        }
    }
}

impl Default for InputService {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn a blocking thread that polls for terminal events and forwards actions to the render loop.
pub fn spawn_input_thread(
    tx: UnboundedSender<InputAction>,
    shutdown: Arc<AtomicBool>,
    poll_interval: Duration,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut service = InputService::new();
        while !shutdown.load(Ordering::SeqCst) {
            match service.poll_actions(Some(poll_interval)) {
                Ok(actions) => {
                    for action in actions {
                        if tx.send(action).is_err() {
                            return;
                        }
                    }
                }
                Err(err) => {
                    eprintln!("Input thread error: {}", err);
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    use std::time::Duration;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn mouse(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn mouse_scrolls_are_coalesced_upstream() {
        let mut service = InputService::new();
        assert!(service
            .process_event(mouse(MouseEventKind::ScrollDown))
            .is_empty());
        assert!(service
            .process_event(mouse(MouseEventKind::ScrollDown))
            .is_empty());
        std::thread::sleep(Duration::from_millis(13));
        let actions = service.process_event(Event::Resize(80, 24));

        assert_eq!(
            actions,
            vec![
                InputAction::Scroll {
                    direction: ScrollDirection::Down,
                    lines: 6,
                },
                InputAction::Resize {
                    width: 80,
                    height: 24,
                },
            ]
        );
    }

    #[test]
    fn keyboard_events_pass_through_state_machine() {
        let mut service = InputService::new();
        let down = service.process_event(key(KeyCode::Char('j')));
        let up = service.process_event(key(KeyCode::Char('k')));

        assert_eq!(
            down,
            vec![InputAction::Scroll {
                direction: ScrollDirection::Down,
                lines: 1,
            }]
        );
        assert_eq!(
            up,
            vec![InputAction::Scroll {
                direction: ScrollDirection::Up,
                lines: 1,
            }]
        );
    }

    #[test]
    fn poll_actions_flushes_pending_events() {
        let mut service = InputService::new();
        service
            .raw_input
            .process_event(mouse(MouseEventKind::ScrollUp));
        std::thread::sleep(Duration::from_millis(13));
        let actions = service
            .poll_actions(Some(Duration::from_millis(1)))
            .unwrap();

        assert_eq!(
            actions,
            vec![InputAction::Scroll {
                direction: ScrollDirection::Up,
                lines: 3,
            }]
        );
    }

    #[test]
    fn mixed_events_preserve_order() {
        let mut service = InputService::new();
        assert!(service
            .process_event(mouse(MouseEventKind::ScrollUp))
            .is_empty());
        std::thread::sleep(Duration::from_millis(13));
        let actions = service.process_event(key(KeyCode::Char('j')));

        assert_eq!(
            actions,
            vec![
                InputAction::Scroll {
                    direction: ScrollDirection::Up,
                    lines: 3,
                },
                InputAction::Scroll {
                    direction: ScrollDirection::Down,
                    lines: 1,
                },
            ]
        );
    }
}
