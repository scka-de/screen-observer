use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::types::{EventType, ObservationEvent, WindowContext};
use crate::ScreenObserver;

/// Default broadcast channel capacity.
const DEFAULT_CHANNEL_CAPACITY: usize = 256;

/// A mock screen observer for testing and demo mode.
///
/// Can be configured to emit a predefined sequence of events
/// at a fixed interval, or to remain silent.
pub struct MockObserver {
    events: Vec<ObservationEvent>,
    interval: Duration,
    sender: broadcast::Sender<ObservationEvent>,
    running: Arc<AtomicBool>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl MockObserver {
    /// Create a mock that emits the given events at the specified interval.
    #[must_use]
    pub fn with_events(events: Vec<ObservationEvent>, interval: Duration) -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_CHANNEL_CAPACITY);
        Self {
            events,
            interval,
            sender,
            running: Arc::new(AtomicBool::new(false)),
            task_handle: None,
        }
    }

    /// Create a mock that never emits any events.
    #[must_use]
    pub fn silent() -> Self {
        Self::with_events(Vec::new(), Duration::from_secs(1))
    }

    /// Helper to build a simple observation event for testing.
    #[must_use]
    pub fn sample_event(app_name: &str, ocr_text: &str) -> ObservationEvent {
        ObservationEvent::new(
            EventType::TextChange,
            WindowContext {
                app_bundle_id: None,
                app_name: app_name.to_string(),
                window_title: None,
                browser_url: None,
                bounding_box: None,
            },
            ocr_text.to_string(),
            0.9,
            true,
        )
    }
}

#[async_trait]
impl ScreenObserver for MockObserver {
    async fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::Acquire) {
            return Ok(());
        }

        self.running.store(true, Ordering::Release);

        let events = self.events.clone();
        let interval = self.interval;
        let sender = self.sender.clone();
        let running = self.running.clone();

        self.task_handle = Some(tokio::spawn(async move {
            for event in events {
                if !running.load(Ordering::Acquire) {
                    break;
                }
                tokio::time::sleep(interval).await;
                if !running.load(Ordering::Acquire) {
                    break;
                }
                // Ignore send errors — no active receivers is fine.
                let _ = sender.send(event);
            }
            // Mark as not running when all events are emitted.
            running.store(false, Ordering::Release);
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

    #[tokio::test]
    async fn mock_emits_events() {
        let events = vec![
            MockObserver::sample_event("Safari", "Hello world"),
            MockObserver::sample_event("Chrome", "Goodbye world"),
            MockObserver::sample_event("Slack", "Meeting at 3pm"),
        ];
        let mut observer = MockObserver::with_events(events, Duration::from_millis(10));
        let mut rx = observer.subscribe();

        observer.start().await.unwrap();

        let mut received = Vec::new();
        for _ in 0..3 {
            let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
                .await
                .expect("timeout waiting for event")
                .expect("channel error");
            received.push(event);
        }

        assert_eq!(received.len(), 3);
        assert_eq!(received[0].window.app_name, "Safari");
        assert_eq!(received[1].window.app_name, "Chrome");
        assert_eq!(received[2].window.app_name, "Slack");

        observer.stop().await.unwrap();
    }

    #[tokio::test]
    async fn mock_stop_prevents_further_events() {
        let events = vec![
            MockObserver::sample_event("App1", "text1"),
            MockObserver::sample_event("App2", "text2"),
        ];
        let mut observer = MockObserver::with_events(events, Duration::from_millis(50));
        let mut rx = observer.subscribe();

        observer.start().await.unwrap();
        assert!(observer.is_running());

        // Wait for and assert first event received.
        let first = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timeout waiting for first event")
            .expect("channel error on first event");
        assert_eq!(first.window.app_name, "App1");

        observer.stop().await.unwrap();
        assert!(!observer.is_running());

        // After stop, no more events should arrive.
        let result = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        assert!(result.is_err(), "should timeout — no events after stop");
    }

    #[tokio::test]
    async fn silent_mock_emits_nothing() {
        let mut observer = MockObserver::silent();
        let mut rx = observer.subscribe();

        observer.start().await.unwrap();

        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(result.is_err(), "silent mock should emit nothing");

        observer.stop().await.unwrap();
    }

    #[tokio::test]
    async fn multiple_subscribers_receive_events() {
        let events = vec![MockObserver::sample_event("Safari", "Hello")];
        let mut observer = MockObserver::with_events(events, Duration::from_millis(10));

        let mut rx1 = observer.subscribe();
        let mut rx2 = observer.subscribe();

        observer.start().await.unwrap();

        let event1 = tokio::time::timeout(Duration::from_secs(1), rx1.recv())
            .await
            .unwrap()
            .unwrap();
        let event2 = tokio::time::timeout(Duration::from_secs(1), rx2.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event1.window.app_name, "Safari");
        assert_eq!(event2.window.app_name, "Safari");
        assert_eq!(event1, event2);

        observer.stop().await.unwrap();
    }

    #[tokio::test]
    async fn start_is_idempotent() {
        let mut observer = MockObserver::silent();
        observer.start().await.unwrap();
        observer.start().await.unwrap(); // should not panic or error
        assert!(observer.is_running());
        observer.stop().await.unwrap();
    }

    #[tokio::test]
    async fn object_safety() {
        // Verify the trait is object-safe: Box<dyn ScreenObserver> compiles.
        let mut observer: Box<dyn ScreenObserver> = Box::new(MockObserver::silent());
        observer.start().await.unwrap();
        assert!(observer.is_running());
        observer.stop().await.unwrap();
    }
}
