use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::broadcast;

use crate::types::{ObservationEvent, EventType, WindowContext};
use crate::ScreenObserver;

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
        let (sender, _) = broadcast::channel(64);
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
            },
            ocr_text.to_string(),
            0.9,
        )
    }
}

impl ScreenObserver for MockObserver {
    async fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.running.store(true, Ordering::SeqCst);

        let events = self.events.clone();
        let interval = self.interval;
        let sender = self.sender.clone();
        let running = self.running.clone();

        self.task_handle = Some(tokio::spawn(async move {
            for event in events {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(interval).await;
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                // Ignore send errors — no active receivers is fine.
                let _ = sender.send(event);
            }
        }));

        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<ObservationEvent> {
        self.sender.subscribe()
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
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
                .expect("channel closed");
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

        // Receive first event.
        let _ = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;

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
        assert_eq!(event1.id, event2.id);

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
}
