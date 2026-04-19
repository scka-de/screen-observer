use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::types::{EventType, ObservationEvent, WindowContext};
use crate::ScreenObserver;

/// Default Screenpipe API base URL.
const DEFAULT_BASE_URL: &str = "http://localhost:3030";

/// How often to poll the Screenpipe search API.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How many recent OCR results to fetch per poll.
const POLL_LIMIT: u32 = 5;

/// Broadcast channel capacity.
const CHANNEL_CAPACITY: usize = 256;

/// Health check timeout.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

// --- Screenpipe API response types ---

#[derive(Debug, Deserialize)]
struct RawContentItem {
    #[serde(rename = "type")]
    content_type: String,
    content: serde_json::Value,
}

enum ContentItem {
    Ocr(OcrContent),
    Other,
}

impl RawContentItem {
    fn parse(&self) -> ContentItem {
        if self.content_type == "OCR" {
            match serde_json::from_value::<OcrContent>(self.content.clone()) {
                Ok(ocr) => ContentItem::Ocr(ocr),
                Err(_) => ContentItem::Other,
            }
        } else {
            ContentItem::Other
        }
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    data: Vec<RawContentItem>,
}

#[derive(Debug, Deserialize)]
struct OcrContent {
    frame_id: i64,
    text: String,
    timestamp: DateTime<Utc>,
    app_name: String,
    window_name: String,
    browser_url: Option<String>,
    focused: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
}

// --- Observer implementation ---

/// Configuration for the Screenpipe observer.
#[derive(Debug, Clone)]
pub struct ScreenpipeConfig {
    /// Base URL of the Screenpipe API (default: `http://localhost:3030`).
    pub base_url: String,
    /// How often to poll for new OCR data.
    pub poll_interval: Duration,
}

impl Default for ScreenpipeConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            poll_interval: POLL_INTERVAL,
        }
    }
}

/// Screen observer backed by a running Screenpipe instance.
///
/// Polls the Screenpipe search API at a configured interval and
/// converts OCR results into `ObservationEvent`s.
pub struct ScreenpipeObserver {
    config: ScreenpipeConfig,
    client: reqwest::Client,
    sender: broadcast::Sender<ObservationEvent>,
    running: Arc<AtomicBool>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
    /// Last `frame_id` seen, to avoid duplicate events.
    last_frame_id: Arc<std::sync::Mutex<i64>>,
}

impl ScreenpipeObserver {
    /// Create a new observer with the given configuration.
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client fails to build (should not happen in practice).
    #[must_use]
    pub fn new(config: ScreenpipeConfig) -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");

        Self {
            config,
            client,
            sender,
            running: Arc::new(AtomicBool::new(false)),
            task_handle: None,
            last_frame_id: Arc::new(std::sync::Mutex::new(0)),
        }
    }

    /// Create a new observer with default configuration.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(ScreenpipeConfig::default())
    }

    /// Check if Screenpipe is healthy and reachable.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response is unparseable.
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/health", self.config.base_url);
        let response = self
            .client
            .get(&url)
            .timeout(HEALTH_TIMEOUT)
            .send()
            .await
            .context("failed to reach Screenpipe")?;

        if !response.status().is_success() {
            return Ok(false);
        }

        let health: HealthResponse = response
            .json()
            .await
            .context("failed to parse health response")?;

        Ok(health.status == "healthy")
    }

    /// Fetch recent OCR results from Screenpipe.
    async fn fetch_recent(&self) -> Result<Vec<ObservationEvent>> {
        let url = format!(
            "{}/search?content_type=ocr&limit={}&offset=0",
            self.config.base_url, POLL_LIMIT
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("failed to fetch from Screenpipe")?;

        let search: SearchResponse = response
            .json()
            .await
            .context("failed to parse search response")?;

        let last_id = *self.last_frame_id.lock().unwrap();
        let mut events = Vec::new();
        let mut max_id = last_id;

        for raw in search.data {
            if let ContentItem::Ocr(ref ocr) = raw.parse() {
                if ocr.frame_id <= last_id {
                    continue;
                }
                if ocr.frame_id > max_id {
                    max_id = ocr.frame_id;
                }
                events.push(ocr_to_event(ocr));
            }
        }

        if max_id > last_id {
            *self.last_frame_id.lock().unwrap() = max_id;
        }

        Ok(events)
    }
}

#[async_trait]
impl ScreenObserver for ScreenpipeObserver {
    async fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::Acquire) {
            return Ok(());
        }

        // Verify Screenpipe is reachable before starting.
        let healthy = self.health_check().await.unwrap_or(false);

        if !healthy {
            anyhow::bail!("Screenpipe is not reachable at {}", self.config.base_url);
        }

        self.running.store(true, Ordering::Release);

        let client = self.client.clone();
        let base_url = self.config.base_url.clone();
        let poll_interval = self.config.poll_interval;
        let sender = self.sender.clone();
        let running = self.running.clone();
        let last_frame_id = self.last_frame_id.clone();

        self.task_handle = Some(tokio::spawn(async move {
            while running.load(Ordering::Acquire) {
                // Fetch recent OCR data.
                let url = format!("{base_url}/search?content_type=ocr&limit={POLL_LIMIT}&offset=0");

                match client.get(&url).send().await {
                    Ok(response) => {
                        if let Ok(search) = response.json::<SearchResponse>().await {
                            let last_id = *last_frame_id.lock().unwrap();
                            let mut max_id = last_id;

                            for raw in search.data {
                                if let ContentItem::Ocr(ocr) = raw.parse() {
                                    if ocr.frame_id <= last_id {
                                        continue;
                                    }
                                    if ocr.frame_id > max_id {
                                        max_id = ocr.frame_id;
                                    }
                                    let event = ocr_to_event(&ocr);
                                    let _ = sender.send(event);
                                }
                            }

                            if max_id > last_id {
                                *last_frame_id.lock().unwrap() = max_id;
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Screenpipe poll failed: {e}");
                    }
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

/// Convert a Screenpipe OCR result to an `ObservationEvent`.
fn ocr_to_event(ocr: &OcrContent) -> ObservationEvent {
    ObservationEvent {
        id: uuid::Uuid::new_v4(),
        timestamp: ocr.timestamp,
        event_type: EventType::TextChange,
        window: WindowContext {
            app_bundle_id: None, // Screenpipe doesn't provide bundle IDs
            app_name: ocr.app_name.clone(),
            window_title: Some(ocr.window_name.clone()),
            browser_url: ocr.browser_url.clone(),
            bounding_box: None,
        },
        ocr_text: ocr.text.chars().take(500).collect(),
        ocr_confidence: 0.9, // Screenpipe doesn't expose per-frame confidence
        is_focused: ocr.focused.unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ocr_content() {
        let json = r#"{
            "type": "OCR",
            "content": {
                "frame_id": 123,
                "text": "Hello world",
                "timestamp": "2026-04-18T14:30:00Z",
                "file_path": "/data/video.mp4",
                "offset_index": 0,
                "app_name": "Safari",
                "window_name": "Google Search",
                "tags": [],
                "frame": null,
                "frame_name": null,
                "browser_url": "https://google.com",
                "focused": true,
                "device_name": "MacBook"
            }
        }"#;

        let raw: RawContentItem = serde_json::from_str(json).unwrap();
        if let ContentItem::Ocr(ocr) = raw.parse() {
            assert_eq!(ocr.app_name, "Safari");
            assert_eq!(ocr.window_name, "Google Search");
            assert_eq!(ocr.text, "Hello world");
            assert_eq!(ocr.frame_id, 123);
            assert_eq!(ocr.browser_url, Some("https://google.com".to_string()));
            assert_eq!(ocr.focused, Some(true));
        } else {
            panic!("expected OCR content");
        }
    }

    #[test]
    fn parse_search_response() {
        let json = r#"{
            "data": [
                {
                    "type": "OCR",
                    "content": {
                        "frame_id": 1,
                        "text": "Hello",
                        "timestamp": "2026-04-18T14:30:00Z",
                        "file_path": "",
                        "offset_index": 0,
                        "app_name": "Safari",
                        "window_name": "Tab1",
                        "tags": [],
                        "device_name": "Mac"
                    }
                },
                {
                    "type": "Audio",
                    "content": {
                        "chunk_id": 2,
                        "transcription": "test",
                        "timestamp": "2026-04-18T14:30:00Z",
                        "file_path": "",
                        "offset_index": 0,
                        "tags": [],
                        "device_name": "mic",
                        "device_type": "Input"
                    }
                }
            ],
            "pagination": {"limit": 5, "offset": 0, "total": 2}
        }"#;

        let response: SearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.data.len(), 2);

        // First item is OCR.
        assert!(matches!(response.data[0].parse(), ContentItem::Ocr(_)));
        // Second item is Audio → Other (we don't care about audio in V0).
        assert!(matches!(response.data[1].parse(), ContentItem::Other));
    }

    #[test]
    fn ocr_to_event_maps_correctly() {
        let ocr = OcrContent {
            frame_id: 42,
            text: "Dear Manager, hope this email finds you well".to_string(),
            timestamp: Utc::now(),
            app_name: "Gmail".to_string(),
            window_name: "Compose".to_string(),
            browser_url: Some("https://mail.google.com".to_string()),
            focused: Some(true),
        };

        let event = ocr_to_event(&ocr);
        assert_eq!(event.window.app_name, "Gmail");
        assert_eq!(event.window.window_title, Some("Compose".to_string()));
        assert_eq!(
            event.window.browser_url,
            Some("https://mail.google.com".to_string())
        );
        assert!(event.is_focused);
        assert!(event.ocr_text.contains("Dear Manager"));
    }

    #[test]
    fn ocr_to_event_truncates_long_text() {
        let long_text = "a".repeat(1000);
        let ocr = OcrContent {
            frame_id: 1,
            text: long_text,
            timestamp: Utc::now(),
            app_name: "App".to_string(),
            window_name: "Win".to_string(),
            browser_url: None,
            focused: None,
        };

        let event = ocr_to_event(&ocr);
        assert_eq!(event.ocr_text.len(), 500);
    }

    #[test]
    fn health_response_parses() {
        let json = r#"{"status": "healthy", "status_code": 200}"#;
        let health: HealthResponse = serde_json::from_str(json).unwrap();
        assert_eq!(health.status, "healthy");
    }

    #[test]
    fn default_config() {
        let config = ScreenpipeConfig::default();
        assert_eq!(config.base_url, "http://localhost:3030");
        assert_eq!(config.poll_interval, Duration::from_secs(1));
    }

    #[tokio::test]
    async fn start_fails_without_screenpipe() {
        let config = ScreenpipeConfig {
            base_url: "http://localhost:19999".to_string(), // unlikely to be running
            poll_interval: Duration::from_millis(100),
        };
        let mut observer = ScreenpipeObserver::new(config);

        let result = observer.start().await;
        assert!(result.is_err());
        assert!(!observer.is_running());
    }
}
