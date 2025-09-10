//! Async event processing system for rlless
//!
//! This module provides a channel-based async event system using crossterm's EventStream
//! and tokio tasks to eliminate blocking event polling and improve responsiveness.

use crate::error::Result;
use crate::ui::{InputAction, InputStateMachine};
use crossterm::event::{Event, EventStream, MouseEvent, MouseEventKind};
use futures::StreamExt;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Application events sent through async channels
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// User input events (keys, mouse, etc.)
    Input(InputAction),
    /// Periodic application updates
    Tick,
    /// UI refresh requests
    Render,
    /// Terminal resize events
    Resize { width: u16, height: u16 },
    /// Shutdown signal
    Quit,
}

/// Event handler managing async tasks and channels
pub struct EventHandler {
    event_tx: mpsc::UnboundedSender<AppEvent>,
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    input_task: Option<JoinHandle<()>>,
    tick_task: Option<JoinHandle<()>>,
    render_task: Option<JoinHandle<()>>,
    cancellation_token: CancellationToken,
}

impl EventHandler {
    /// Create a new event handler with unbounded channel
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let cancellation_token = CancellationToken::new();
        
        Self {
            event_tx,
            event_rx,
            input_task: None,
            tick_task: None,
            render_task: None,
            cancellation_token,
        }
    }
    
    /// Start the input processing task
    pub fn start_input_task(&mut self, input_machine: InputStateMachine) -> Result<()> {
        let tx = self.event_tx.clone();
        let token = self.cancellation_token.clone();
        
        self.input_task = Some(tokio::spawn(input_handler_task(tx, input_machine, token)));
        Ok(())
    }
    
    /// Start the periodic tick task
    pub fn start_tick_task(&mut self, tick_rate: Duration) -> Result<()> {
        let tx = self.event_tx.clone();
        let token = self.cancellation_token.clone();
        
        self.tick_task = Some(tokio::spawn(tick_handler_task(tx, tick_rate, token)));
        Ok(())
    }
    
    /// Start the render refresh task
    pub fn start_render_task(&mut self, render_rate: Duration) -> Result<()> {
        let tx = self.event_tx.clone();
        let token = self.cancellation_token.clone();
        
        self.render_task = Some(tokio::spawn(render_handler_task(tx, render_rate, token)));
        Ok(())
    }
    
    /// Receive the next event from the channel
    pub async fn next_event(&mut self) -> Option<AppEvent> {
        self.event_rx.recv().await
    }
    
    /// Shutdown all async tasks gracefully
    pub async fn shutdown(&mut self) {
        // Signal all tasks to stop
        self.cancellation_token.cancel();
        
        // Wait for tasks to complete
        if let Some(task) = self.input_task.take() {
            let _ = task.await;
        }
        if let Some(task) = self.tick_task.take() {
            let _ = task.await;
        }
        if let Some(task) = self.render_task.take() {
            let _ = task.await;
        }
    }
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Async task for handling crossterm input events
pub async fn input_handler_task(
    event_tx: mpsc::UnboundedSender<AppEvent>,
    mut input_machine: InputStateMachine,
    cancellation_token: CancellationToken,
) {
    let mut event_stream = EventStream::new();
    let mut mouse_throttle = MouseThrottle::new();
    
    loop {
        tokio::select! {
            // Handle crossterm events non-blocking
            Some(crossterm_event) = event_stream.next() => {
                match crossterm_event {
                    Ok(Event::Key(key_event)) => {
                        let action = input_machine.handle_key_event(key_event);
                        if !matches!(action, InputAction::NoAction) {
                            let _ = event_tx.send(AppEvent::Input(action));
                        }
                    },
                    Ok(Event::Resize(w, h)) => {
                        let _ = event_tx.send(AppEvent::Resize { width: w, height: h });
                    },
                    Ok(Event::Mouse(mouse)) => {
                        if let Some(action) = mouse_throttle.handle_event(mouse) {
                            let _ = event_tx.send(AppEvent::Input(action));
                        }
                    },
                    Err(_) => break, // Stream error, exit task
                    _ => {} // Ignore other events
                }
            },
            // Graceful shutdown
            _ = cancellation_token.cancelled() => break,
        }
    }
}

/// Async task for periodic application updates
pub async fn tick_handler_task(
    event_tx: mpsc::UnboundedSender<AppEvent>,
    tick_rate: Duration,
    cancellation_token: CancellationToken,
) {
    let mut interval = tokio::time::interval(tick_rate);
    
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let _ = event_tx.send(AppEvent::Tick);
            },
            _ = cancellation_token.cancelled() => break,
        }
    }
}

/// Async task for UI refresh events
pub async fn render_handler_task(
    event_tx: mpsc::UnboundedSender<AppEvent>,
    render_rate: Duration,
    cancellation_token: CancellationToken,
) {
    let mut interval = tokio::time::interval(render_rate);
    
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let _ = event_tx.send(AppEvent::Render);
            },
            _ = cancellation_token.cancelled() => break,
        }
    }
}

/// Mouse event throttling to prevent overwhelming the system
struct MouseThrottle {
    last_scroll_time: Option<Instant>,
    throttle_duration: Duration,
}

impl MouseThrottle {
    /// Create new mouse throttle with 100ms throttle duration
    fn new() -> Self {
        Self {
            last_scroll_time: None,
            throttle_duration: Duration::from_millis(100),
        }
    }
    
    /// Handle mouse event with throttling applied
    fn handle_event(&mut self, mouse_event: MouseEvent) -> Option<InputAction> {
        match mouse_event.kind {
            MouseEventKind::ScrollUp => {
                if self.should_throttle() {
                    return None;
                }
                self.update_time();
                // Mouse scroll up = move up in file (show earlier lines)
                Some(InputAction::ScrollUp(3))
            }
            MouseEventKind::ScrollDown => {
                if self.should_throttle() {
                    return None;
                }
                self.update_time();
                // Mouse scroll down = move down in file (show later lines)
                Some(InputAction::ScrollDown(3))
            }
            _ => {
                // Ignore other mouse events (clicks, moves, etc.)
                None
            }
        }
    }
    
    /// Check if scroll event should be throttled
    fn should_throttle(&self) -> bool {
        if let Some(last_time) = self.last_scroll_time {
            let now = Instant::now();
            now.duration_since(last_time) < self.throttle_duration
        } else {
            false
        }
    }
    
    /// Update the last scroll time
    fn update_time(&mut self) {
        self.last_scroll_time = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;
    
    #[tokio::test]
    async fn test_event_handler_creation() {
        let event_handler = EventHandler::new();
        
        // Should be able to create without error
        assert!(event_handler.input_task.is_none());
        assert!(event_handler.tick_task.is_none());
        assert!(event_handler.render_task.is_none());
    }
    
    #[tokio::test]
    async fn test_tick_task() {
        let mut event_handler = EventHandler::new();
        
        // Start tick task with short interval
        event_handler.start_tick_task(Duration::from_millis(10)).unwrap();
        
        // Should receive tick events
        let event = timeout(Duration::from_millis(50), event_handler.next_event())
            .await
            .unwrap()
            .unwrap();
        
        assert!(matches!(event, AppEvent::Tick));
        
        // Clean shutdown
        event_handler.shutdown().await;
    }
    
    #[tokio::test]
    async fn test_render_task() {
        let mut event_handler = EventHandler::new();
        
        // Start render task with short interval
        event_handler.start_render_task(Duration::from_millis(10)).unwrap();
        
        // Should receive render events
        let event = timeout(Duration::from_millis(50), event_handler.next_event())
            .await
            .unwrap()
            .unwrap();
        
        assert!(matches!(event, AppEvent::Render));
        
        // Clean shutdown
        event_handler.shutdown().await;
    }
    
    #[test]
    fn test_mouse_throttle() {
        use crossterm::event::{MouseEventKind, MouseEvent};
        
        let mut throttle = MouseThrottle::new();
        
        // First scroll should work
        let mouse_event = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::empty(),
        };
        
        let action = throttle.handle_event(mouse_event);
        assert!(matches!(action, Some(InputAction::ScrollUp(3))));
        
        // Immediate second scroll should be throttled
        let action = throttle.handle_event(mouse_event);
        assert!(action.is_none());
    }
    
    #[tokio::test]
    async fn test_graceful_shutdown() {
        let mut event_handler = EventHandler::new();
        
        // Start all tasks
        let input_machine = InputStateMachine::new();
        event_handler.start_input_task(input_machine).unwrap();
        event_handler.start_tick_task(Duration::from_millis(100)).unwrap();
        event_handler.start_render_task(Duration::from_millis(100)).unwrap();
        
        // All tasks should be running
        assert!(event_handler.input_task.is_some());
        assert!(event_handler.tick_task.is_some());
        assert!(event_handler.render_task.is_some());
        
        // Shutdown should complete without hanging
        let shutdown_result = timeout(Duration::from_millis(500), event_handler.shutdown()).await;
        assert!(shutdown_result.is_ok());
        
        // All tasks should be cleaned up
        assert!(event_handler.input_task.is_none());
        assert!(event_handler.tick_task.is_none());
        assert!(event_handler.render_task.is_none());
    }
}