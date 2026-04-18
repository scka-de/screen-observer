use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Why this observation event was triggered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventType {
    /// User switched to a different application.
    AppSwitch,
    /// Text content on screen changed significantly.
    TextChange,
    /// User paused typing for a detectable interval.
    TypingPause,
    /// User scrolled and stopped.
    ScrollStop,
    /// User clicked.
    Click,
    /// Clipboard content changed.
    Clipboard,
    /// No user activity detected — periodic idle capture.
    Idle,
}

/// Metadata about the active window at observation time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowContext {
    /// Application bundle identifier (e.g., "com.apple.Safari").
    pub app_bundle_id: Option<String>,
    /// Human-readable application name (e.g., "Safari").
    pub app_name: String,
    /// Window title (e.g., "Inbox - Gmail").
    pub window_title: Option<String>,
    /// Browser URL if applicable.
    pub browser_url: Option<String>,
}

/// A single observation event from the screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationEvent {
    /// Unique identifier for this event.
    pub id: Uuid,
    /// When the observation was captured.
    pub timestamp: DateTime<Utc>,
    /// What triggered this observation.
    pub event_type: EventType,
    /// Context about the active window.
    pub window: WindowContext,
    /// OCR-extracted text from the screen (truncated to 500 chars).
    pub ocr_text: String,
    /// Confidence of the OCR extraction (0.0 to 1.0).
    pub ocr_confidence: f64,
    /// Whether this window is currently focused.
    pub is_focused: bool,
}

impl ObservationEvent {
    /// Create a new observation event with a generated ID and current timestamp.
    #[must_use]
    pub fn new(
        event_type: EventType,
        window: WindowContext,
        ocr_text: String,
        ocr_confidence: f64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            event_type,
            window,
            ocr_text,
            ocr_confidence,
            is_focused: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_window() -> WindowContext {
        WindowContext {
            app_bundle_id: Some("com.apple.Safari".to_string()),
            app_name: "Safari".to_string(),
            window_title: Some("Inbox - Gmail".to_string()),
            browser_url: Some("https://mail.google.com".to_string()),
        }
    }

    fn sample_event() -> ObservationEvent {
        ObservationEvent::new(
            EventType::TextChange,
            sample_window(),
            "Dear Manager, I hope this email finds you well...".to_string(),
            0.95,
        )
    }

    #[test]
    fn event_has_unique_id() {
        let a = sample_event();
        let b = sample_event();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn event_has_timestamp() {
        let event = sample_event();
        let now = Utc::now();
        let diff = now - event.timestamp;
        assert!(diff.num_seconds() < 1);
    }

    #[test]
    fn event_serializes_to_json() {
        let event = sample_event();
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("TextChange"));
        assert!(json.contains("Safari"));
        assert!(json.contains("Dear Manager"));
    }

    #[test]
    fn event_roundtrips_through_json() {
        let event = sample_event();
        let json = serde_json::to_string(&event).unwrap();
        let restored: ObservationEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, event.id);
        assert_eq!(restored.event_type, event.event_type);
        assert_eq!(restored.window.app_name, "Safari");
        assert_eq!(restored.ocr_text, event.ocr_text);
    }

    #[test]
    fn window_context_optional_fields() {
        let window = WindowContext {
            app_bundle_id: None,
            app_name: "Unknown".to_string(),
            window_title: None,
            browser_url: None,
        };
        let event = ObservationEvent::new(EventType::Idle, window, String::new(), 0.0);
        assert!(event.window.app_bundle_id.is_none());
        assert!(event.window.window_title.is_none());
    }
}
