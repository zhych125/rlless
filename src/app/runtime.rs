use crate::input::{InputAction, InputService};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// Spawn a blocking thread that collects terminal input and forwards actions onto a channel.
pub fn spawn_input_thread(
    tx: UnboundedSender<InputAction>,
    shutdown: Arc<AtomicBool>,
    poll_interval: Duration,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut service = InputService::new();
        while !shutdown.load(Ordering::SeqCst) {
            match service.poll_action(Some(poll_interval)) {
                Ok(Some(action)) => {
                    if tx.send(action).is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    // No input this tick; continue polling.
                    continue;
                }
                Err(err) => {
                    eprintln!("Input thread error: {}", err);
                    break;
                }
            }
        }
    })
}
