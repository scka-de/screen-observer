pub mod capture;
pub mod frame_diff;
pub mod ocr;
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

use frame_diff::{FrameDiffConfig, FrameDiffer};

/// Broadcast channel capacity.
const CHANNEL_CAPACITY: usize = 256;

/// Default polling interval.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Configuration for the native macOS observer.
#[derive(Debug, Clone)]
pub struct NativeConfig {
    /// How often to poll for screen changes.
    pub poll_interval: Duration,
    /// Frame diff configuration.
    pub frame_diff: FrameDiffConfig,
}

impl Default for NativeConfig {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            frame_diff: FrameDiffConfig::default(),
        }
    }
}

/// Native macOS screen observer using `ScreenCaptureKit` + Vision OCR.
///
/// Captures the screen, detects changes via frame diff, runs OCR on
/// changed frames, and emits `ObservationEvent`s. No external
/// dependencies required — only macOS Screen Recording permission.
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
        if permissions::check_screen_recording() == permissions::PermissionStatus::Denied
            && permissions::request_screen_recording() == permissions::PermissionStatus::Denied
        {
            anyhow::bail!(
                "Screen Recording permission denied. \
                 Enable it in System Settings → Privacy & Security → Screen Recording."
            );
        }

        self.running.store(true, Ordering::Release);

        let poll_interval = self.config.poll_interval;
        let diff_config = self.config.frame_diff.clone();
        let sender = self.sender.clone();
        let running = self.running.clone();

        self.task_handle = Some(tokio::spawn(async move {
            run_loop(running, sender, poll_interval, diff_config).await;
        }));

        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Release);
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
            let _ = handle.await; // drive to completion, ignore JoinError
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

/// Main observation loop: window detection → capture → diff → OCR → emit.
async fn run_loop(
    running: Arc<AtomicBool>,
    sender: broadcast::Sender<ObservationEvent>,
    poll_interval: Duration,
    diff_config: FrameDiffConfig,
) {
    let mut differ = FrameDiffer::new(diff_config);
    let mut last_app: Option<String> = None;
    let mut last_title = String::new();

    while running.load(Ordering::Acquire) {
        // Step 1: Get focused window metadata.
        let Some(info) = window_info::get_focused_window() else {
            tokio::time::sleep(poll_interval).await;
            continue;
        };

        let current_title = info.window_title.as_deref().unwrap_or("");

        // Determine event type from a single decision tree.
        let event_type = match &last_app {
            None => EventType::FocusChange,
            Some(prev) if *prev != info.app_name => EventType::AppSwitch,
            Some(_) if current_title != last_title => EventType::FocusChange,
            _ => EventType::TextChange,
        };
        let window_changed = event_type != EventType::TextChange;

        last_app = Some(info.app_name.clone());
        last_title.clear();
        last_title.push_str(current_title);

        // Step 2: Capture screenshot.
        let frame = match capture::capture_screen() {
            Ok(img) => img,
            Err(e) => {
                log::warn!("Screen capture failed: {e}");
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        };

        // Step 3: Frame diff — skip OCR if frame hasn't changed and window is the same.
        if !window_changed && !differ.has_changed(&frame) {
            tokio::time::sleep(poll_interval).await;
            continue;
        }

        // Update differ state on window change.
        if window_changed {
            differ.has_changed(&frame); // reset baseline
        }

        // Step 4: OCR via spawn_blocking (Vision is synchronous).
        let ocr_result =
            match tokio::task::spawn_blocking(move || ocr::recognize_text(&frame)).await {
                Ok(Ok(result)) => result,
                Ok(Err(e)) => {
                    log::warn!("OCR failed: {e}");
                    ocr::OcrResult {
                        text: String::new(),
                        confidence: 0.0,
                    }
                }
                Err(e) => {
                    log::error!("OCR task panicked: {e}");
                    ocr::OcrResult {
                        text: String::new(),
                        confidence: 0.0,
                    }
                }
            };

        // Step 5: Emit event.
        let event = ObservationEvent::new(
            event_type,
            WindowContext::from(&info),
            ocr_result.text,
            ocr_result.confidence,
            info.is_on_screen,
        );

        let _ = sender.send(event);

        tokio::time::sleep(poll_interval).await;
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
    fn default_config_includes_frame_diff() {
        let config = NativeConfig::default();
        assert!((config.frame_diff.change_threshold - 0.02).abs() < f64::EPSILON);
        assert_eq!(config.frame_diff.force_interval_secs, 10);
    }

    #[test]
    fn observer_starts_not_running() {
        let observer = NativeObserver::new(NativeConfig::default());
        assert!(!observer.is_running());
    }

    #[test]
    fn observer_is_object_safe() {
        fn _takes_observer(_: Box<dyn ScreenObserver>) {}
    }

    #[tokio::test]
    async fn start_is_idempotent() {
        let mut observer = NativeObserver::new(NativeConfig::default());
        // Can't actually start without Screen Recording permission,
        // but calling start twice should not panic.
        let _ = observer.start().await;
        let _ = observer.start().await;
    }

    #[tokio::test]
    async fn stop_without_start_is_safe() {
        let mut observer = NativeObserver::new(NativeConfig::default());
        let result = observer.stop().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore = "requires macOS with Screen Recording permission"]
    async fn observer_emits_events_with_ocr() {
        let config = NativeConfig {
            poll_interval: Duration::from_millis(500),
            ..NativeConfig::default()
        };
        let mut observer = NativeObserver::new(config);
        let mut rx = observer.subscribe();

        observer.start().await.unwrap();
        assert!(observer.is_running());

        // Wait for first event (may take a few seconds for capture + OCR).
        let event = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("timed out waiting for event")
            .expect("channel closed");

        assert!(!event.window.app_name.is_empty(), "app_name should be set");
        // OCR may or may not find text depending on screen content,
        // but confidence should be in valid range.
        assert!(
            event.ocr_confidence >= 0.0 && event.ocr_confidence <= 1.0,
            "confidence out of range: {}",
            event.ocr_confidence
        );

        observer.stop().await.unwrap();
        assert!(!observer.is_running());
    }

    #[tokio::test]
    #[ignore = "requires macOS with Screen Recording permission"]
    async fn observer_detects_window_change() {
        let config = NativeConfig {
            poll_interval: Duration::from_millis(200),
            ..NativeConfig::default()
        };
        let mut observer = NativeObserver::new(config);
        let mut rx = observer.subscribe();

        observer.start().await.unwrap();

        let event = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        // First event should be FocusChange (initial detection).
        assert_eq!(event.event_type, EventType::FocusChange);

        observer.stop().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires macOS with Screen Recording permission"]
    async fn multiple_subscribers_receive_events() {
        let config = NativeConfig {
            poll_interval: Duration::from_millis(500),
            ..NativeConfig::default()
        };
        let mut observer = NativeObserver::new(config);
        let mut rx1 = observer.subscribe();
        let mut rx2 = observer.subscribe();

        observer.start().await.unwrap();

        let e1 = tokio::time::timeout(Duration::from_secs(10), rx1.recv()).await;
        let e2 = tokio::time::timeout(Duration::from_secs(10), rx2.recv()).await;

        assert!(e1.is_ok(), "subscriber 1 should receive event");
        assert!(e2.is_ok(), "subscriber 2 should receive event");

        observer.stop().await.unwrap();
    }
}
