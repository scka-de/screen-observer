pub mod permissions;
pub mod window_info;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::types::{EventType, ObservationEvent, WindowContext};
use crate::ScreenObserver;

/// Broadcast channel capacity.
const CHANNEL_CAPACITY: usize = 256;

/// Default polling interval.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Configuration for the native macOS observer.
#[derive(Debug, Clone)]
pub struct NativeConfig {
    /// How often to poll for screen changes.
    pub poll_interval: Duration,
}

impl Default for NativeConfig {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

/// Native macOS screen observer using `ScreenCaptureKit` + Vision OCR.
///
/// Directly captures the screen and runs OCR without external dependencies.
/// Requires Screen Recording permission on macOS.
pub struct NativeObserver {
    config: NativeConfig,
    sender: broadcast::Sender<ObservationEvent>,
    running: Arc<AtomicBool>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl NativeObserver {
    /// Create a new native observer with the given configuration.
    #[must_use]
    pub fn new(config: NativeConfig) -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);

        Self {
            config,
            sender,
            running: Arc::new(AtomicBool::new(false)),
            task_handle: None,
        }
    }
}

#[async_trait]
impl ScreenObserver for NativeObserver {
    async fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::Acquire) {
            return Ok(());
        }

        // Check screen recording permission.
        if permissions::check_screen_recording() == permissions::PermissionStatus::Denied {
            // Try requesting it (shows system dialog).
            if permissions::request_screen_recording() == permissions::PermissionStatus::Denied {
                anyhow::bail!(
                    "Screen Recording permission denied. \
                     Enable it in System Settings → Privacy & Security → Screen Recording."
                );
            }
        }

        self.running.store(true, Ordering::Release);

        let poll_interval = self.config.poll_interval;
        let sender = self.sender.clone();
        let running = self.running.clone();

        self.task_handle = Some(tokio::spawn(async move {
            let mut last_app = String::new();
            let mut last_title = String::new();

            while running.load(Ordering::Acquire) {
                if let Some(info) = window_info::get_focused_window() {
                    let current_title = info.window_title.as_deref().unwrap_or("");

                    // Determine event type based on window change.
                    // Skip if nothing changed — don't flood downstream.
                    let event_type = if last_app.is_empty() {
                        EventType::FocusChange
                    } else if info.app_name != last_app {
                        EventType::AppSwitch
                    } else if current_title != last_title {
                        EventType::FocusChange
                    } else {
                        // Nothing changed — skip this tick.
                        tokio::time::sleep(poll_interval).await;
                        continue;
                    };

                    last_app.clone_from(&info.app_name);
                    last_title = current_title.to_string();

                    let event = ObservationEvent::new(
                        event_type,
                        WindowContext::from(&info),
                        String::new(), // OCR text added in Phase N3
                        0.0,           // OCR confidence added in Phase N3
                        info.is_on_screen,
                    );

                    let _ = sender.send(event);
                }

                tokio::time::sleep(poll_interval).await;
            }
        }));

        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Release);
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<ObservationEvent> {
        self.sender.subscribe()
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = NativeConfig::default();
        assert_eq!(config.poll_interval, Duration::from_secs(1));
    }

    #[test]
    fn observer_starts_not_running() {
        let observer = NativeObserver::new(NativeConfig::default());
        assert!(!observer.is_running());
    }

    #[test]
    fn observer_is_object_safe() {
        // Verify the trait can be used as a trait object.
        fn _takes_observer(_: Box<dyn ScreenObserver>) {}
    }

    #[tokio::test]
    #[ignore = "requires macOS with Screen Recording permission"]
    async fn observer_emits_events() {
        let config = NativeConfig {
            poll_interval: Duration::from_millis(200),
        };
        let mut observer = NativeObserver::new(config);
        let mut rx = observer.subscribe();

        observer.start().await.unwrap();
        assert!(observer.is_running());

        let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timed out waiting for event")
            .expect("channel closed");

        assert!(!event.window.app_name.is_empty());

        observer.stop().await.unwrap();
        assert!(!observer.is_running());
    }
}
