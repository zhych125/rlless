# Async Event Handling Design Plan

## Overview
This document outlines the design for replacing the current synchronous event polling with an async channel-based event system using crossterm's async EventStream and tokio tasks.

## Current Problems
- Blocking UI thread during `crossterm::event::poll()` with 100ms timeout
- Application loop uses busy sleep pattern (`tokio::time::sleep(10ms)`)
- Fixed timeout can cause input lag
- No concurrent event processing capabilities

## Proposed Architecture

### 1. Event Types and Channels

```rust
// Internal application events (sent through channels)
#[derive(Debug, Clone)]
pub enum AppEvent {
    Input(InputAction),       // User input events (keys, mouse, etc.)
    Tick,                     // Periodic application updates (100ms)
    Render,                   // UI refresh requests (60fps)
    Resize { width: u16, height: u16 },  // Terminal resize events
    Quit,                     // Shutdown signal
}

// Event handler manages async tasks and channels
pub struct EventHandler {
    event_tx: mpsc::UnboundedSender<AppEvent>,
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    input_task: Option<JoinHandle<()>>,
    tick_task: Option<JoinHandle<()>>,
    render_task: Option<JoinHandle<()>>,
    cancellation_token: CancellationToken,
}
```

### 2. Input Processing Task

```rust
// Separate async task for handling crossterm events
pub async fn input_handler_task(
    event_tx: mpsc::UnboundedSender<AppEvent>,
    mut input_machine: InputStateMachine,
    cancellation_token: CancellationToken,
) {
    let mut event_stream = EventStream::new();
    
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
                        if let Some(action) = handle_mouse_event_async(mouse) {
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
```

### 3. Periodic Task Handlers

```rust
// Tick task for application updates
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

// Render task for UI refresh
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
```

### 4. Updated Application Loop

```rust
// In Application::run()
pub async fn run(&mut self) -> Result<()> {
    // Initialize UI first
    self.ui_renderer.initialize()?;
    
    // Create event handler
    let mut event_handler = EventHandler::new();
    
    // Start async event processing tasks
    event_handler.start_input_task(self.ui_renderer.get_input_machine())?;
    event_handler.start_tick_task(Duration::from_millis(100))?;  // 10fps for updates
    event_handler.start_render_task(Duration::from_millis(16))?; // 60fps for rendering
    
    // Initialize view state
    let (width, height) = self.ui_renderer.get_terminal_size()?;
    let file_path = self.file_accessor.file_path().to_path_buf();
    let mut view_state = ViewState::new(file_path, width, height);
    self.update_view_content(&mut view_state, false).await?;
    
    // Main event loop - now channel-based, non-blocking
    let mut running = true;
    while running {
        match event_handler.next_event().await {
            Some(AppEvent::Input(action)) => {
                running = self.execute_action(action, &mut view_state).await?;
            },
            Some(AppEvent::Tick) => {
                // Periodic application updates (search progress, file changes, etc.)
                // Could be used for background search operations
            },
            Some(AppEvent::Render) => {
                // Separate render timing from input/logic
                self.ui_renderer.render(&view_state)?;
            },
            Some(AppEvent::Resize { width, height }) => {
                if view_state.update_terminal_size(width, height) {
                    self.update_view_content(&mut view_state, self.search_state.is_some()).await?;
                }
            },
            Some(AppEvent::Quit) | None => {
                running = false;
            }
        }
    }
    
    // Clean shutdown
    event_handler.shutdown().await;
    self.ui_renderer.cleanup()?;
    Ok(())
}
```

### 5. EventHandler Implementation

```rust
impl EventHandler {
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
    
    pub fn start_input_task(&mut self, input_machine: InputStateMachine) -> Result<()> {
        let tx = self.event_tx.clone();
        let token = self.cancellation_token.clone();
        
        self.input_task = Some(tokio::spawn(input_handler_task(tx, input_machine, token)));
        Ok(())
    }
    
    pub fn start_tick_task(&mut self, tick_rate: Duration) -> Result<()> {
        let tx = self.event_tx.clone();
        let token = self.cancellation_token.clone();
        
        self.tick_task = Some(tokio::spawn(tick_handler_task(tx, tick_rate, token)));
        Ok(())
    }
    
    pub fn start_render_task(&mut self, render_rate: Duration) -> Result<()> {
        let tx = self.event_tx.clone();
        let token = self.cancellation_token.clone();
        
        self.render_task = Some(tokio::spawn(render_handler_task(tx, render_rate, token)));
        Ok(())
    }
    
    pub async fn next_event(&mut self) -> Option<AppEvent> {
        self.event_rx.recv().await
    }
    
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
```

## Migration Strategy

### Step 1: Preparation (No Breaking Changes)
- Add new dependencies to Cargo.toml
- Create event_handler.rs and mouse_throttle.rs modules
- Add async event infrastructure alongside existing code
- Ensure existing tests still pass

### Step 2: Parallel Implementation
- Implement EventHandler with channel-based architecture
- Create async version of main loop as `run_async()`
- Keep existing `run()` method working
- Add feature flag to switch between implementations

### Step 3: Testing & Validation
- Test async implementation thoroughly
- Compare performance and responsiveness
- Ensure no regressions in existing functionality
- Validate mouse event throttling works correctly

### Step 4: Migration
- Switch main.rs to use `run_async()`
- Remove old synchronous event handling code
- Clean up unused methods and state
- Update tests to work with new architecture

### Step 5: Cleanup
- Remove deprecated methods and traits
- Update documentation
- Optimize performance based on real usage

## Expected Benefits

### Performance Improvements:
- **Mouse lag elimination**: Events processed immediately (0ms vs 50ms delay)
- **Better responsiveness**: No polling timeout delays
- **Smoother scrolling**: Events arrive at natural mouse/touchpad frequency
- **Lower CPU usage**: Event-driven vs busy polling

### Architecture Benefits:
- **Cleaner separation**: Event handling separated from UI rendering
- **Better scalability**: Can handle high-frequency events without blocking
- **Future extensibility**: Easy to add new event types
- **Modern async patterns**: Leverages tokio ecosystem

## Risks and Mitigation

### Risk 1: Complexity Increase
- **Mitigation**: Implement incrementally, keep existing code until new system proven
- **Testing**: Extensive testing of async behavior and edge cases

### Risk 2: Multithreading Bugs
- **Mitigation**: Use proven patterns from ratatui async template
- **Design**: Keep UI rendering on main thread, only event processing async

### Risk 3: Performance Regression
- **Mitigation**: Benchmark before/after, maintain performance tests
- **Monitoring**: Add metrics to track event processing latency

## Success Metrics

### Quantitative:
- Mouse event latency: Target <10ms (from current 50-60ms)
- CPU usage: Should decrease due to eliminating polling
- Memory usage: Should remain stable
- All existing tests pass

### Qualitative:
- Mouse scrolling feels immediately responsive
- No noticeable lag between scroll gesture and content movement
- Smooth integration with existing keyboard navigation
- No UI glitches or race conditions

## Timeline Estimate

- **Phase 1** (Dependencies): 1 hour
- **Phase 2** (Event Infrastructure): 4-6 hours  
- **Phase 3** (Main Loop): 2-3 hours
- **Phase 4** (UI Updates): 2-3 hours
- **Phase 5** (Trait Updates): 1-2 hours
- **Testing & Refinement**: 3-4 hours

**Total**: 13-19 hours over multiple sessions

## Files to be Modified

### New Files:
- `src/ui/event_handler.rs` - Async event processing
- `src/ui/mouse_throttle.rs` - Self-contained mouse throttling

### Modified Files:
- `Cargo.toml` - Add async dependencies
- `src/app.rs` - New async main loop
- `src/ui/terminal.rs` - Remove event handling, keep rendering
- `src/ui/renderer.rs` - Update trait interface
- `src/main.rs` - Switch to async runtime

### Test Files:
- Update existing tests for new architecture
- Add async event handling tests
- Benchmark tests for performance validation

This plan provides a comprehensive roadmap for eliminating mouse event lag while maintaining system stability and adding beneficial architectural improvements.