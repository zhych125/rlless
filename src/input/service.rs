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
    Command,
    PercentInput,
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
    StartCommand,
    UpdateCommandBuffer(String),
    CancelCommand,
    ExecuteCommand {
        buffer: String,
    },
    StartPercentInput,
    UpdatePercentBuffer(String),
    CancelPercentInput,
    SubmitPercent(u8),
    NoAction,
    InvalidInput,
}

/// State machine that mirrors classic `less` bindings.
pub struct InputStateMachine {
    state: InputState,
    search_buffer: String,
    command_buffer: String,
    percent_buffer: String,
    search_history: Vec<String>,
    history_cursor: Option<usize>,
}

impl InputStateMachine {
    pub fn new() -> Self {
        Self {
            state: InputState::Navigation,
            search_buffer: String::new(),
            command_buffer: String::new(),
            percent_buffer: String::new(),
            search_history: Vec::new(),
            history_cursor: None,
        }
    }

    fn clear_percent_buffer(&mut self) {
        self.percent_buffer.clear();
    }

    pub fn handle_key_event(&mut self, key_event: KeyEvent) -> InputAction {
        if key_event.kind != KeyEventKind::Press {
            return InputAction::NoAction;
        }

        match (self.state, key_event.code, key_event.modifiers) {
            (InputState::Navigation, KeyCode::Char('%'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.state = InputState::PercentInput;
                self.clear_percent_buffer();
                InputAction::StartPercentInput
            }
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
            (InputState::Navigation, KeyCode::Char('-'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.state = InputState::Command;
                self.command_buffer.clear();
                InputAction::StartCommand
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
                self.history_cursor = None;
                InputAction::StartSearch(SearchDirection::Forward)
            }
            (InputState::Navigation, KeyCode::Char('?'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.state = InputState::SearchInput {
                    direction: SearchDirection::Backward,
                };
                self.search_buffer.clear();
                self.history_cursor = None;
                InputAction::StartSearch(SearchDirection::Backward)
            }
            (InputState::SearchInput { .. }, KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.state = InputState::Navigation;
                self.search_buffer.clear();
                self.history_cursor = None;
                InputAction::CancelSearch
            }
            (InputState::SearchInput { direction }, KeyCode::Char(ch), modifiers)
                if (ch.is_ascii_graphic() || ch == ' ')
                    && !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.history_cursor = None;
                self.search_buffer.push(ch);
                InputAction::UpdateSearchBuffer {
                    direction,
                    buffer: self.search_buffer.clone(),
                }
            }
            (InputState::SearchInput { direction }, KeyCode::Backspace, _) => {
                self.history_cursor = None;
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
                self.history_cursor = None;

                if pattern.trim().is_empty() {
                    InputAction::CancelSearch
                } else {
                    let trimmed = pattern.trim().to_string();
                    self.record_history(&trimmed);
                    InputAction::ExecuteSearch {
                        pattern: trimmed,
                        direction,
                    }
                }
            }
            (InputState::SearchInput { .. }, KeyCode::Esc, _) => {
                self.state = InputState::Navigation;
                self.search_buffer.clear();
                self.history_cursor = None;
                InputAction::CancelSearch
            }
            (InputState::SearchInput { direction }, KeyCode::Up, _) => {
                if self.search_history.is_empty() {
                    return InputAction::NoAction;
                }

                let next_index = match self.history_cursor {
                    None => self.search_history.len().saturating_sub(1),
                    Some(0) => 0,
                    Some(idx) => idx.saturating_sub(1),
                };

                self.history_cursor = Some(next_index);
                if let Some(entry) = self.search_history.get(next_index) {
                    self.search_buffer = entry.clone();
                }
                InputAction::UpdateSearchBuffer {
                    direction,
                    buffer: self.search_buffer.clone(),
                }
            }
            (InputState::SearchInput { direction }, KeyCode::Down, _) => {
                if self.search_history.is_empty() {
                    return InputAction::NoAction;
                }

                match self.history_cursor {
                    None => InputAction::NoAction,
                    Some(idx) if idx + 1 < self.search_history.len() => {
                        let next_index = idx + 1;
                        self.history_cursor = Some(next_index);
                        if let Some(entry) = self.search_history.get(next_index) {
                            self.search_buffer = entry.clone();
                        }
                        InputAction::UpdateSearchBuffer {
                            direction,
                            buffer: self.search_buffer.clone(),
                        }
                    }
                    Some(_) => {
                        self.history_cursor = None;
                        self.search_buffer.clear();
                        InputAction::UpdateSearchBuffer {
                            direction,
                            buffer: self.search_buffer.clone(),
                        }
                    }
                }
            }
            (InputState::Command, KeyCode::Esc, _)
            | (InputState::Command, KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.state = InputState::Navigation;
                self.command_buffer.clear();
                InputAction::CancelCommand
            }
            (InputState::Command, KeyCode::Enter, _) => {
                let buffer = self.command_buffer.clone();
                self.state = InputState::Navigation;
                self.command_buffer.clear();
                InputAction::ExecuteCommand { buffer }
            }
            (InputState::Command, KeyCode::Backspace, _) => {
                if self.command_buffer.pop().is_some() {
                    InputAction::UpdateCommandBuffer(self.command_buffer.clone())
                } else {
                    self.state = InputState::Navigation;
                    InputAction::CancelCommand
                }
            }
            (InputState::Command, KeyCode::Char(ch), modifiers)
                if (ch.is_ascii_graphic() || ch == ' ')
                    && !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.command_buffer.push(ch);
                InputAction::UpdateCommandBuffer(self.command_buffer.clone())
            }
            (InputState::Command, _, _) => InputAction::InvalidInput,
            (InputState::PercentInput, KeyCode::Char(ch @ '0'..='9'), modifiers)
                if !modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if self.percent_buffer.len() < 3 {
                    self.percent_buffer.push(ch);
                }
                InputAction::UpdatePercentBuffer(self.percent_buffer.clone())
            }
            (InputState::PercentInput, KeyCode::Backspace, _) => {
                if self.percent_buffer.pop().is_some() {
                    InputAction::UpdatePercentBuffer(self.percent_buffer.clone())
                } else {
                    self.state = InputState::Navigation;
                    InputAction::CancelPercentInput
                }
            }
            (InputState::PercentInput, KeyCode::Enter, _) => {
                let buffer = self.percent_buffer.clone();
                self.clear_percent_buffer();
                self.state = InputState::Navigation;

                if buffer.is_empty() {
                    return InputAction::CancelPercentInput;
                }

                match buffer.parse::<u16>() {
                    Ok(value) => InputAction::SubmitPercent(value.min(100) as u8),
                    Err(_) => InputAction::InvalidInput,
                }
            }
            (InputState::PercentInput, KeyCode::Esc, _) => {
                self.clear_percent_buffer();
                self.state = InputState::Navigation;
                InputAction::CancelPercentInput
            }
            (InputState::PercentInput, KeyCode::Char('c'), modifiers)
                if modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.clear_percent_buffer();
                self.state = InputState::Navigation;
                InputAction::CancelPercentInput
            }
            (InputState::PercentInput, _, _) => InputAction::InvalidInput,
            _ => {
                self.clear_percent_buffer();
                InputAction::InvalidInput
            }
        }
    }

    pub fn get_search_buffer(&self) -> &str {
        &self.search_buffer
    }

    pub fn get_state(&self) -> InputState {
        self.state
    }

    fn record_history(&mut self, pattern: &str) {
        if pattern.is_empty() {
            return;
        }
        if self
            .search_history
            .last()
            .is_some_and(|last| last == pattern)
        {
            return;
        }
        self.search_history.push(pattern.to_string());
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

    #[test]
    fn percent_jump_emits_action() {
        let mut service = InputService::new();

        assert_eq!(
            service.process_event(key(KeyCode::Char('%'))),
            vec![InputAction::StartPercentInput]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Char('5'))),
            vec![InputAction::UpdatePercentBuffer("5".to_string())]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Char('0'))),
            vec![InputAction::UpdatePercentBuffer("50".to_string())]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Enter)),
            vec![InputAction::SubmitPercent(50)]
        );
    }

    #[test]
    fn percent_jump_backspace_cancels_when_empty() {
        let mut service = InputService::new();

        assert_eq!(
            service.process_event(key(KeyCode::Char('%'))),
            vec![InputAction::StartPercentInput]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Char('1'))),
            vec![InputAction::UpdatePercentBuffer("1".to_string())]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Backspace)),
            vec![InputAction::UpdatePercentBuffer(String::new())]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Backspace)),
            vec![InputAction::CancelPercentInput]
        );
    }

    fn ctrl_char(ch: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL))
    }

    #[test]
    fn percent_jump_ctrl_c_cancels() {
        let mut service = InputService::new();

        assert_eq!(
            service.process_event(key(KeyCode::Char('%'))),
            vec![InputAction::StartPercentInput]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Char('2'))),
            vec![InputAction::UpdatePercentBuffer("2".to_string())]
        );

        assert_eq!(
            service.process_event(ctrl_char('c')),
            vec![InputAction::CancelPercentInput]
        );
    }

    #[test]
    fn percent_jump_escape_cancels() {
        let mut service = InputService::new();

        assert_eq!(
            service.process_event(key(KeyCode::Char('%'))),
            vec![InputAction::StartPercentInput]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Esc)),
            vec![InputAction::CancelPercentInput]
        );
    }

    #[test]
    fn search_history_navigation_allows_recall() {
        let mut service = InputService::new();

        // First search: "/f"
        service.process_event(key(KeyCode::Char('/')));
        service.process_event(key(KeyCode::Char('f')));
        service.process_event(key(KeyCode::Enter));

        // Second search: "/bar"
        service.process_event(key(KeyCode::Char('/')));
        service.process_event(key(KeyCode::Char('b')));
        service.process_event(key(KeyCode::Char('a')));
        service.process_event(key(KeyCode::Char('r')));
        service.process_event(key(KeyCode::Enter));

        // Start a new search session to recall history
        assert_eq!(
            service.process_event(key(KeyCode::Char('/'))),
            vec![InputAction::StartSearch(SearchDirection::Forward)]
        );

        // Up -> recalls most recent entry "bar"
        assert_eq!(
            service.process_event(key(KeyCode::Up)),
            vec![InputAction::UpdateSearchBuffer {
                direction: SearchDirection::Forward,
                buffer: "bar".to_string(),
            }]
        );

        // Another Up -> older entry "f"
        assert_eq!(
            service.process_event(key(KeyCode::Up)),
            vec![InputAction::UpdateSearchBuffer {
                direction: SearchDirection::Forward,
                buffer: "f".to_string(),
            }]
        );

        // Down -> returns to "bar"
        assert_eq!(
            service.process_event(key(KeyCode::Down)),
            vec![InputAction::UpdateSearchBuffer {
                direction: SearchDirection::Forward,
                buffer: "bar".to_string(),
            }]
        );

        // Down past latest entry -> clears buffer
        assert_eq!(
            service.process_event(key(KeyCode::Down)),
            vec![InputAction::UpdateSearchBuffer {
                direction: SearchDirection::Forward,
                buffer: String::new(),
            }]
        );

        // Typing after recall resets the cursor
        assert_eq!(
            service.process_event(key(KeyCode::Char('z'))),
            vec![InputAction::UpdateSearchBuffer {
                direction: SearchDirection::Forward,
                buffer: "z".to_string(),
            }]
        );

        // Up again recalls the latest history entry
        assert_eq!(
            service.process_event(key(KeyCode::Up)),
            vec![InputAction::UpdateSearchBuffer {
                direction: SearchDirection::Forward,
                buffer: "bar".to_string(),
            }]
        );
    }

    #[test]
    fn command_mode_updates_buffer_and_executes() {
        let mut service = InputService::new();

        assert_eq!(
            service.process_event(key(KeyCode::Char('-'))),
            vec![InputAction::StartCommand]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Char('i'))),
            vec![InputAction::UpdateCommandBuffer("i".to_string())]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Backspace)),
            vec![InputAction::UpdateCommandBuffer(String::new())]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Char('r'))),
            vec![InputAction::UpdateCommandBuffer("r".to_string())]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Enter)),
            vec![InputAction::ExecuteCommand {
                buffer: "r".to_string(),
            }]
        );
    }

    #[test]
    fn command_mode_cancel_clears_buffer() {
        let mut service = InputService::new();

        assert_eq!(
            service.process_event(key(KeyCode::Char('-'))),
            vec![InputAction::StartCommand]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Char('w'))),
            vec![InputAction::UpdateCommandBuffer("w".to_string())]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Esc)),
            vec![InputAction::CancelCommand]
        );
    }

    #[test]
    fn command_mode_backspace_when_empty_exits() {
        let mut service = InputService::new();

        assert_eq!(
            service.process_event(key(KeyCode::Char('-'))),
            vec![InputAction::StartCommand]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Backspace)),
            vec![InputAction::CancelCommand]
        );

        assert_eq!(
            service.process_event(key(KeyCode::Char('-'))),
            vec![InputAction::StartCommand]
        );
    }
}
