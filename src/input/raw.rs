//! Low-level input collection: crossterm polling translated into primitive events for the
//! higher-level input service.
//!
//! This module now owns scroll coalescing, so repeated wheel events are merged before they reach
//! the state machine.

use crate::error::Result;
use crate::input::ScrollDirection;
use ratatui::crossterm::event::{self, Event, KeyEvent, MouseEvent, MouseEventKind};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Number of lines represented by a single mouse wheel tick.
const MOUSE_SCROLL_LINES: u64 = 1;
/// Poll timeout used when the caller does not provide one. Matched to the render cadence (~60 Hz).
const DEFAULT_POLL_TIMEOUT_MS: u64 = 16;
/// Default coalescing window in milliseconds for scroll bursts.
const DEFAULT_COALESCE_WINDOW_MS: u64 = 12;

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

/// Collector that polls crossterm for events, performs scroll coalescing, and queues them for
/// higher-level processing.
pub struct RawInputCollector {
    scroll_coalescer: ScrollCoalescer,
    pending_events: VecDeque<RawInputEvent>,
}

impl RawInputCollector {
    /// Create a collector with an empty queue.
    pub fn new() -> Self {
        Self {
            scroll_coalescer: ScrollCoalescer::with_default_window(),
            pending_events: VecDeque::new(),
        }
    }

    /// Check whether the collector has any queued events or pending scroll aggregation.
    pub fn is_idle(&self) -> bool {
        self.pending_events.is_empty() && self.scroll_coalescer.is_empty()
    }

    /// Process a synthetic event (primarily used by unit tests).
    pub fn process_event(&mut self, event: Event) {
        self.enqueue_event(event);
    }

    /// Retrieve the next raw input event, blocking up to `timeout`.
    pub fn poll_event(&mut self, timeout: Option<Duration>) -> Result<Option<RawInputEvent>> {
        if let Some(event) = self.try_flush_scroll() {
            return Ok(Some(event));
        }

        let poll_timeout = timeout.unwrap_or(Duration::from_millis(DEFAULT_POLL_TIMEOUT_MS));

        if !event::poll(poll_timeout)? {
            return Ok(self.try_flush_scroll());
        }

        let event = event::read()?;
        self.enqueue_event(event);
        Ok(self.try_flush_scroll())
    }

    /// Pop the next pending raw event from the queue.
    pub fn pop_pending(&mut self) -> Option<RawInputEvent> {
        self.pending_events.pop_front()
    }

    /// Try to flush a coalesced scroll if the window has expired, otherwise pop the next queued
    /// event. Exposed for callers that want to drain without blocking.
    pub fn try_flush(&mut self) -> Option<RawInputEvent> {
        self.try_flush_scroll()
    }

    fn enqueue_event(&mut self, event: Event) {
        match event {
            Event::Key(key_event) => {
                self.flush_scroll();
                self.pending_events.push_back(RawInputEvent::Key(key_event));
            }
            Event::Resize(width, height) => {
                self.flush_scroll();
                self.pending_events
                    .push_back(RawInputEvent::Resize { width, height });
            }
            Event::Mouse(mouse_event) => self.queue_scroll(mouse_event),
            _ => {}
        }
    }

    fn queue_scroll(&mut self, mouse_event: MouseEvent) {
        let direction = match mouse_event.kind {
            MouseEventKind::ScrollUp => ScrollDirection::Up,
            MouseEventKind::ScrollDown => ScrollDirection::Down,
            _ => return,
        };

        let now = Instant::now();
        if let Some((dir, lines)) = self
            .scroll_coalescer
            .push(direction, MOUSE_SCROLL_LINES, now)
        {
            self.pending_events.push_back(RawInputEvent::Scroll {
                direction: dir,
                lines,
            });
        }
    }

    fn flush_scroll(&mut self) {
        if let Some((dir, lines)) = self.scroll_coalescer.flush() {
            self.pending_events.push_back(RawInputEvent::Scroll {
                direction: dir,
                lines,
            });
        }
    }

    fn try_flush_scroll(&mut self) -> Option<RawInputEvent> {
        if let Some((dir, lines)) = self.scroll_coalescer.flush_if_stale(Instant::now()) {
            return Some(RawInputEvent::Scroll {
                direction: dir,
                lines,
            });
        }
        self.pop_pending()
    }
}

impl Default for RawInputCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
struct ScrollCoalescer {
    window: Duration,
    pending: Option<PendingScroll>,
}

#[derive(Debug, Clone)]
struct PendingScroll {
    direction: ScrollDirection,
    lines: u64,
    last_event: Instant,
}

impl ScrollCoalescer {
    fn with_default_window() -> Self {
        Self::new(Duration::from_millis(DEFAULT_COALESCE_WINDOW_MS))
    }

    fn new(window: Duration) -> Self {
        Self {
            window,
            pending: None,
        }
    }

    fn push(
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

    fn flush_if_stale(&mut self, now: Instant) -> Option<(ScrollDirection, u64)> {
        if let Some(pending) = &self.pending {
            if now.duration_since(pending.last_event) >= self.window {
                return self.flush();
            }
        }
        None
    }

    fn flush(&mut self) -> Option<(ScrollDirection, u64)> {
        self.pending
            .take()
            .map(|pending| (pending.direction, pending.lines))
    }

    fn is_empty(&self) -> bool {
        self.pending.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    use std::time::Duration;

    fn key_press(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn mouse_scroll_direction_change_flushes() {
        let mut collector = RawInputCollector::new();

        collector.process_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        collector.process_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));

        let first = collector.try_flush().unwrap();
        assert_eq!(
            first,
            RawInputEvent::Scroll {
                direction: ScrollDirection::Down,
                lines: MOUSE_SCROLL_LINES,
            }
        );

        std::thread::sleep(Duration::from_millis(DEFAULT_COALESCE_WINDOW_MS + 1));
        let second = collector.try_flush().unwrap();
        assert_eq!(
            second,
            RawInputEvent::Scroll {
                direction: ScrollDirection::Up,
                lines: MOUSE_SCROLL_LINES,
            }
        );
    }

    #[test]
    fn coalesces_same_direction_scrolls() {
        let mut collector = RawInputCollector::new();

        collector.process_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        collector.process_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));

        std::thread::sleep(Duration::from_millis(DEFAULT_COALESCE_WINDOW_MS + 1));
        let flushed = collector.try_flush().unwrap();
        assert_eq!(
            flushed,
            RawInputEvent::Scroll {
                direction: ScrollDirection::Down,
                lines: MOUSE_SCROLL_LINES * 2,
            }
        );
    }

    #[test]
    fn resize_flushes_pending_scroll() {
        let mut collector = RawInputCollector::new();

        collector.process_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        collector.process_event(Event::Resize(120, 40));

        let first = collector.try_flush().unwrap();
        assert!(matches!(first, RawInputEvent::Scroll { .. }));
        let second = collector.try_flush().unwrap();
        assert_eq!(
            second,
            RawInputEvent::Resize {
                width: 120,
                height: 40,
            }
        );
    }

    #[test]
    fn queues_key_events() {
        let mut collector = RawInputCollector::new();
        collector.process_event(key_press(KeyCode::Char('j')));

        let result = collector.try_flush().unwrap();
        match result {
            RawInputEvent::Key(key) => assert_eq!(key.code, KeyCode::Char('j')),
            _ => panic!("expected key event"),
        }
    }
}
