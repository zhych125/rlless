//! Low-level input collection: crossterm polling, mouse scroll coalescing, and
//! translation into primitive events that the higher-level input service can consume.

use crate::error::Result;
use ratatui::crossterm::event::{self, Event, KeyEvent, MouseEvent, MouseEventKind};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Default coalescing window in milliseconds for scroll events.
const DEFAULT_COALESCE_WINDOW_MS: u64 = 12;
/// Number of lines produced by a single mouse wheel tick.
const MOUSE_SCROLL_LINES: u64 = 3;
/// Poll timeout used when the caller does not provide one.
const DEFAULT_POLL_TIMEOUT_MS: u64 = 50;

/// Direction for scroll coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
}

/// Low-level events surfaced by the raw input collector.
#[derive(Debug, Clone, PartialEq)]
pub enum RawInputEvent {
    Key(KeyEvent),
    Resize {
        width: u16,
        height: u16,
    },
    Scroll {
        direction: ScrollDirection,
        lines: u64,
    },
}

/// Aggregates high-frequency scroll events into larger steps.
#[derive(Debug, Clone)]
pub struct InputCoalescer {
    window: Duration,
    pending: Option<PendingScroll>,
}

#[derive(Debug, Clone)]
struct PendingScroll {
    direction: ScrollDirection,
    lines: u64,
    last_event: Instant,
}

impl InputCoalescer {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            pending: None,
        }
    }

    pub fn with_default_window() -> Self {
        Self::new(Duration::from_millis(DEFAULT_COALESCE_WINDOW_MS))
    }

    /// Register a new scroll event, returning any previously queued scroll that should be flushed.
    pub fn push(
        &mut self,
        direction: ScrollDirection,
        lines: u64,
        now: Instant,
    ) -> Option<(ScrollDirection, u64)> {
        match self.pending {
            None => {
                self.pending = Some(PendingScroll {
                    direction,
                    lines,
                    last_event: now,
                });
                None
            }
            Some(ref mut pending) if pending.direction == direction => {
                pending.lines = pending.lines.saturating_add(lines);
                pending.last_event = now;
                None
            }
            Some(_) => {
                let flushed = self.flush();
                self.pending = Some(PendingScroll {
                    direction,
                    lines,
                    last_event: now,
                });
                flushed
            }
        }
    }

    /// Flush accumulated scrolls if the coalescing window has expired.
    pub fn flush_if_stale(&mut self, now: Instant) -> Option<(ScrollDirection, u64)> {
        if let Some(pending) = &self.pending {
            if now.duration_since(pending.last_event) >= self.window {
                return self.flush();
            }
        }
        None
    }

    /// Flush all accumulated scrolls immediately.
    pub fn flush(&mut self) -> Option<(ScrollDirection, u64)> {
        self.pending
            .take()
            .map(|pending| (pending.direction, pending.lines))
    }

    /// Return true when there is no pending scroll to be flushed.
    pub fn is_empty(&self) -> bool {
        self.pending.is_none()
    }
}

/// Collector that polls crossterm for events and applies scroll coalescing.
#[derive(Debug)]
pub struct RawInputCollector {
    coalescer: InputCoalescer,
    pending_events: VecDeque<RawInputEvent>,
}

impl RawInputCollector {
    /// Create a collector with the default coalescing window.
    pub fn new() -> Self {
        Self {
            coalescer: InputCoalescer::with_default_window(),
            pending_events: VecDeque::new(),
        }
    }

    /// Create a collector with a custom coalescing window (useful for tests).
    pub fn with_window(window: Duration) -> Self {
        Self {
            coalescer: InputCoalescer::new(window),
            pending_events: VecDeque::new(),
        }
    }

    /// Check whether the collector has no pending events or scroll accumulation.
    pub fn is_idle(&self) -> bool {
        self.pending_events.is_empty() && self.coalescer.is_empty()
    }

    /// Process a synthetic event (primarily used by unit tests).
    pub fn process_event(&mut self, event: Event) {
        self.enqueue_event(event);
    }

    /// Attempt to flush coalesced scrolls without polling crossterm.
    pub fn try_flush_coalesced(&mut self) -> Option<RawInputEvent> {
        self.coalescer
            .flush_if_stale(Instant::now())
            .map(|(direction, lines)| RawInputEvent::Scroll { direction, lines })
            .or_else(|| self.pop_pending())
    }

    /// Retrieve the next raw input event, blocking up to `timeout`.
    pub fn poll_event(&mut self, timeout: Option<Duration>) -> Result<Option<RawInputEvent>> {
        if let Some(event) = self.try_flush_coalesced() {
            return Ok(Some(event));
        }

        let poll_timeout = timeout.unwrap_or(Duration::from_millis(DEFAULT_POLL_TIMEOUT_MS));

        if !event::poll(poll_timeout)? {
            if let Some(event) = self.try_flush_coalesced() {
                return Ok(Some(event));
            }
            return Ok(None);
        }

        let event = event::read()?;
        self.enqueue_event(event);
        Ok(self.pop_pending())
    }

    fn enqueue_event(&mut self, event: Event) {
        match event {
            Event::Key(key_event) => {
                self.pending_events.push_back(RawInputEvent::Key(key_event));
            }
            Event::Resize(width, height) => {
                self.flush_pending_scroll();
                self.pending_events
                    .push_back(RawInputEvent::Resize { width, height });
            }
            Event::Mouse(mouse_event) => {
                if let Some(scroll) = self.handle_mouse_event(mouse_event) {
                    self.pending_events.push_back(scroll);
                }
            }
            _ => {}
        }
    }

    fn handle_mouse_event(&mut self, mouse_event: MouseEvent) -> Option<RawInputEvent> {
        let direction = match mouse_event.kind {
            MouseEventKind::ScrollUp => ScrollDirection::Up,
            MouseEventKind::ScrollDown => ScrollDirection::Down,
            _ => return None,
        };

        let now = Instant::now();
        if let Some((flushed_dir, lines)) = self.coalescer.push(direction, MOUSE_SCROLL_LINES, now)
        {
            self.pending_events.push_back(RawInputEvent::Scroll {
                direction: flushed_dir,
                lines,
            });
        }

        None
    }

    fn flush_pending_scroll(&mut self) {
        if let Some((direction, lines)) = self.coalescer.flush() {
            self.pending_events
                .push_back(RawInputEvent::Scroll { direction, lines });
        }
    }

    /// Pop the next pending raw event without touching the coalescer.
    pub fn pop_pending(&mut self) -> Option<RawInputEvent> {
        self.pending_events.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key_press(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn accumulates_same_direction_within_window() {
        let mut collector = RawInputCollector::with_window(Duration::from_millis(10));
        let now = Instant::now();

        collector.coalescer.push(ScrollDirection::Down, 1, now);
        collector
            .coalescer
            .push(ScrollDirection::Down, 2, now + Duration::from_millis(5));

        std::thread::sleep(Duration::from_millis(11));
        let event = collector.try_flush_coalesced().unwrap();
        assert!(matches!(
            event,
            RawInputEvent::Scroll {
                direction: ScrollDirection::Down,
                lines: 3
            }
        ));
    }

    #[test]
    fn flushes_on_direction_change() {
        let mut collector = RawInputCollector::with_window(Duration::from_millis(10));
        let now = Instant::now();

        collector.coalescer.push(ScrollDirection::Up, 1, now);
        let flushed = collector
            .coalescer
            .push(ScrollDirection::Down, 1, now + Duration::from_millis(3))
            .unwrap();
        assert_eq!(flushed, (ScrollDirection::Up, 1));

        std::thread::sleep(Duration::from_millis(12));
        let event = collector.try_flush_coalesced().unwrap();
        assert!(matches!(
            event,
            RawInputEvent::Scroll {
                direction: ScrollDirection::Down,
                lines: 1
            }
        ));
    }

    #[test]
    fn handles_resize_after_scroll() {
        let mut collector = RawInputCollector::new();

        collector.process_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        collector.process_event(Event::Resize(80, 40));

        // First flush should emit the scroll
        let first = collector.try_flush_coalesced().unwrap();
        assert!(matches!(first, RawInputEvent::Scroll { .. }));

        // Next event is the resize
        let second = collector.try_flush_coalesced().unwrap();
        assert_eq!(
            second,
            RawInputEvent::Resize {
                width: 80,
                height: 40
            }
        );
    }

    #[test]
    fn queues_key_events() {
        let mut collector = RawInputCollector::new();
        collector.process_event(key_press(KeyCode::Char('j')));

        let result = collector.try_flush_coalesced().unwrap();
        match result {
            RawInputEvent::Key(key) => assert_eq!(key.code, KeyCode::Char('j')),
            _ => panic!("expected key event"),
        }
    }
}
