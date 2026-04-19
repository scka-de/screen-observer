use anyhow::{Context, Result};
use image::DynamicImage;

/// Capture a screenshot of the primary monitor.
///
/// # Errors
///
/// Returns an error if no monitors are found or capture fails.
/// Fails silently (returns error) if Screen Recording permission is denied.
pub fn capture_screen() -> Result<DynamicImage> {
    let monitors = xcap::Monitor::all().context("failed to enumerate monitors")?;

    let monitor = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
        .context("no monitors found")?;

    let rgba_image = monitor
        .capture_image()
        .context("failed to capture screenshot (Screen Recording permission needed)")?;

    Ok(DynamicImage::ImageRgba8(rgba_image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires macOS with Screen Recording permission"]
    fn capture_screen_returns_image() {
        let img = capture_screen().unwrap();
        assert!(img.width() > 0);
        assert!(img.height() > 0);
    }

    #[test]
    #[ignore = "requires macOS with Screen Recording permission"]
    fn capture_screen_has_reasonable_dimensions() {
        let img = capture_screen().unwrap();
        // Any real monitor should be at least 800x600.
        assert!(img.width() >= 800, "width too small: {}", img.width());
        assert!(img.height() >= 600, "height too small: {}", img.height());
    }

    #[test]
    #[ignore = "requires macOS with Screen Recording permission"]
    fn two_captures_produce_similar_images() {
        let img1 = capture_screen().unwrap();
        let img2 = capture_screen().unwrap();
        // Dimensions should be the same (same monitor).
        assert_eq!(img1.width(), img2.width());
        assert_eq!(img1.height(), img2.height());
    }
}
