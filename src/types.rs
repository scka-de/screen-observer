use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum length for OCR text. Longer text is truncated in the constructor.
const MAX_OCR_TEXT_CHARS: usize = 500;

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
    /// Window focus changed within the same app or across apps.
    FocusChange,
    /// A keyboard shortcut was detected (e.g., Cmd+Z).
    KeyboardShortcut,
    /// Accessibility tree changed meaningfully (UI state transition).
    AccessibilityChange,
}

/// Bounding box of a window on screen, in screen coordinates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoundingBox {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
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
    /// Window position and size on screen.
    pub bounding_box: Option<BoundingBox>,
}

/// A single observation event from the screen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    ///
    /// `ocr_text` is automatically truncated to 500 characters if longer.
    #[must_use]
    pub fn new(
        event_type: EventType,
        window: WindowContext,
        ocr_text: String,
        ocr_confidence: f64,
        is_focused: bool,
    ) -> Self {
        let ocr_text = if ocr_text.chars().count() > MAX_OCR_TEXT_CHARS {
            ocr_text.chars().take(MAX_OCR_TEXT_CHARS).collect()
        } else {
            ocr_text
        };

        Self {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            event_type,
            window,
            ocr_text,
            ocr_confidence,
            is_focused,
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
            bounding_box: Some(BoundingBox {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            }),
        }
    }

    fn sample_event() -> ObservationEvent {
        ObservationEvent::new(
            EventType::TextChange,
            sample_window(),
            "Dear Manager, I hope this email finds you well...".to_string(),
            0.95,
            true,
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
        assert_eq!(restored, event);
    }

    #[test]
    fn window_context_optional_fields() {
        let window = WindowContext {
            app_bundle_id: None,
            app_name: "Unknown".to_string(),
            window_title: None,
            browser_url: None,
            bounding_box: None,
        };
        let event = ObservationEvent::new(EventType::Idle, window, String::new(), 0.0, false);
        assert!(event.window.app_bundle_id.is_none());
        assert!(event.window.window_title.is_none());
        assert!(event.window.bounding_box.is_none());
        assert!(!event.is_focused);
    }

    #[test]
    fn ocr_text_truncated_to_500_chars() {
        let long_text = "a".repeat(1000);
        let event =
            ObservationEvent::new(EventType::TextChange, sample_window(), long_text, 0.9, true);
        assert_eq!(event.ocr_text.chars().count(), MAX_OCR_TEXT_CHARS);
    }

    #[test]
    fn ocr_text_truncation_preserves_utf8() {
        // Each emoji is 1 char but multiple bytes.
        let emoji_text = "🎭".repeat(600);
        let event = ObservationEvent::new(
            EventType::TextChange,
            sample_window(),
            emoji_text,
            0.9,
            true,
        );
        assert_eq!(event.ocr_text.chars().count(), MAX_OCR_TEXT_CHARS);
        // Verify it's valid UTF-8 (would panic if not).
        let _ = event.ocr_text.as_bytes();
    }

    #[test]
    fn short_ocr_text_not_truncated() {
        let short_text = "Hello world".to_string();
        let event = ObservationEvent::new(
            EventType::TextChange,
            sample_window(),
            short_text.clone(),
            0.9,
            true,
        );
        assert_eq!(event.ocr_text, short_text);
    }

    #[test]
    fn event_types_cover_all_variants() {
        // Ensure all variants serialize correctly.
        let variants = vec![
            EventType::AppSwitch,
            EventType::TextChange,
            EventType::TypingPause,
            EventType::ScrollStop,
            EventType::Click,
            EventType::Clipboard,
            EventType::Idle,
            EventType::FocusChange,
            EventType::KeyboardShortcut,
            EventType::AccessibilityChange,
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).unwrap();
            let restored: EventType = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, variant);
        }
    }
}
