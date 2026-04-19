use image::DynamicImage;

/// Default threshold below which frames are considered unchanged.
const DEFAULT_CHANGE_THRESHOLD: f64 = 0.02;

/// Default downscale factor for comparison (4x reduces 1920→480).
const DEFAULT_DOWNSCALE_FACTOR: u32 = 4;

/// Force a capture after this many seconds even if frame hasn't changed.
const DEFAULT_FORCE_INTERVAL_SECS: u64 = 10;

/// Configuration for frame comparison.
#[derive(Debug, Clone)]
pub struct FrameDiffConfig {
    /// Hellinger distance below this = no change.
    pub change_threshold: f64,
    /// Downscale images by this factor before comparing.
    pub downscale_factor: u32,
    /// Force capture after this many seconds of no change.
    pub force_interval_secs: u64,
}

impl Default for FrameDiffConfig {
    fn default() -> Self {
        Self {
            change_threshold: DEFAULT_CHANGE_THRESHOLD,
            downscale_factor: DEFAULT_DOWNSCALE_FACTOR,
            force_interval_secs: DEFAULT_FORCE_INTERVAL_SECS,
        }
    }
}

/// Compares consecutive frames to detect meaningful screen changes.
///
/// Uses grayscale histogram comparison (Hellinger distance) on
/// downscaled images for performance. Tracks time since last
/// change to force periodic captures.
pub struct FrameDiffer {
    config: FrameDiffConfig,
    previous: Option<DynamicImage>,
    last_processed: std::time::Instant,
}

impl FrameDiffer {
    /// Create a new frame differ with the given config.
    #[must_use]
    pub fn new(config: FrameDiffConfig) -> Self {
        Self {
            config,
            previous: None,
            last_processed: std::time::Instant::now(),
        }
    }

    /// Check if the current frame has changed enough to warrant processing.
    ///
    /// Returns `true` if:
    /// - This is the first frame (no previous)
    /// - The Hellinger distance exceeds the threshold
    /// - The force interval has elapsed (safety valve)
    pub fn has_changed(&mut self, current: &DynamicImage) -> bool {
        // First frame — always process.
        let Some(ref previous) = self.previous else {
            self.previous = Some(current.clone());
            self.last_processed = std::time::Instant::now();
            return true;
        };

        // Force capture after interval (safety valve for slow-changing screens).
        if self.last_processed.elapsed().as_secs() >= self.config.force_interval_secs {
            self.previous = Some(current.clone());
            self.last_processed = std::time::Instant::now();
            return true;
        }

        // Downscale both images for faster comparison.
        let prev_small = downscale(previous, self.config.downscale_factor);
        let curr_small = downscale(current, self.config.downscale_factor);

        // Compare histograms.
        let distance = compare_histograms(&prev_small, &curr_small);

        if distance >= self.config.change_threshold {
            self.previous = Some(current.clone());
            self.last_processed = std::time::Instant::now();
            true
        } else {
            false
        }
    }
}

/// Downscale an image by the given factor using nearest-neighbor.
fn downscale(img: &DynamicImage, factor: u32) -> DynamicImage {
    let new_w = (img.width() / factor).max(1);
    let new_h = (img.height() / factor).max(1);
    img.resize_exact(new_w, new_h, image::imageops::FilterType::Nearest)
}

/// Compare two images via grayscale histogram Hellinger distance.
///
/// Returns a value in [0.0, 1.0] where 0.0 = identical, 1.0 = completely different.
fn compare_histograms(a: &DynamicImage, b: &DynamicImage) -> f64 {
    let gray_a = a.to_luma8();
    let mut gray_b = b.to_luma8();

    // Resize b to match a if dimensions differ.
    if gray_a.dimensions() != gray_b.dimensions() {
        gray_b = image::imageops::resize(
            &gray_b,
            gray_a.width(),
            gray_a.height(),
            image::imageops::FilterType::Nearest,
        );
    }

    image_compare::gray_similarity_histogram(image_compare::Metric::Hellinger, &gray_a, &gray_b)
        .unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    /// Create a solid-color test image.
    fn solid_image(width: u32, height: u32, color: [u8; 4]) -> DynamicImage {
        let mut img = RgbaImage::new(width, height);
        for pixel in img.pixels_mut() {
            *pixel = Rgba(color);
        }
        DynamicImage::ImageRgba8(img)
    }

    /// Create a half-and-half image (left half one color, right half another).
    fn split_image(width: u32, height: u32, left: [u8; 4], right: [u8; 4]) -> DynamicImage {
        let mut img = RgbaImage::new(width, height);
        for (x, _, pixel) in img.enumerate_pixels_mut() {
            *pixel = if x < width / 2 {
                Rgba(left)
            } else {
                Rgba(right)
            };
        }
        DynamicImage::ImageRgba8(img)
    }

    // --- FrameDiffConfig tests ---

    #[test]
    fn default_config_values() {
        let config = FrameDiffConfig::default();
        assert!((config.change_threshold - 0.02).abs() < f64::EPSILON);
        assert_eq!(config.downscale_factor, 4);
        assert_eq!(config.force_interval_secs, 10);
    }

    // --- FrameDiffer::has_changed tests ---

    #[test]
    fn first_frame_always_changed() {
        let mut differ = FrameDiffer::new(FrameDiffConfig::default());
        let img = solid_image(100, 100, [128, 128, 128, 255]);
        assert!(
            differ.has_changed(&img),
            "first frame should always be a change"
        );
    }

    #[test]
    fn identical_frame_not_changed() {
        let mut differ = FrameDiffer::new(FrameDiffConfig::default());
        let img = solid_image(100, 100, [128, 128, 128, 255]);

        assert!(differ.has_changed(&img)); // first
        assert!(
            !differ.has_changed(&img),
            "identical frame should not be a change"
        );
    }

    #[test]
    fn very_different_frame_is_changed() {
        let mut differ = FrameDiffer::new(FrameDiffConfig::default());
        let black = solid_image(100, 100, [0, 0, 0, 255]);
        let white = solid_image(100, 100, [255, 255, 255, 255]);

        assert!(differ.has_changed(&black)); // first
        assert!(differ.has_changed(&white), "black→white should be a change");
    }

    #[test]
    fn identical_frame_stays_unchanged() {
        // Uniform images with exact same values: histogram distance = 0.
        let mut differ = FrameDiffer::new(FrameDiffConfig::default());
        let gray = solid_image(100, 100, [128, 128, 128, 255]);

        assert!(differ.has_changed(&gray)); // first
        assert!(
            !differ.has_changed(&gray),
            "exact same image should be below threshold"
        );
    }

    #[test]
    fn moderate_change_above_threshold_detected() {
        let mut differ = FrameDiffer::new(FrameDiffConfig::default());
        let dark = solid_image(200, 200, [50, 50, 50, 255]);
        let bright = solid_image(200, 200, [200, 200, 200, 255]);

        assert!(differ.has_changed(&dark)); // first
        assert!(
            differ.has_changed(&bright),
            "large brightness shift should exceed threshold"
        );
    }

    #[test]
    fn partial_change_detected() {
        let mut differ = FrameDiffer::new(FrameDiffConfig::default());
        let all_black = solid_image(200, 200, [0, 0, 0, 255]);
        let half_white = split_image(200, 200, [0, 0, 0, 255], [255, 255, 255, 255]);

        assert!(differ.has_changed(&all_black)); // first
        assert!(
            differ.has_changed(&half_white),
            "50% of pixels changing should be detected"
        );
    }

    #[test]
    fn force_capture_after_interval() {
        let mut differ = FrameDiffer::new(FrameDiffConfig {
            force_interval_secs: 0, // force immediately
            ..FrameDiffConfig::default()
        });
        let img = solid_image(100, 100, [128, 128, 128, 255]);

        assert!(differ.has_changed(&img)); // first
                                           // Even though frame is identical, force interval of 0 means always force.
        assert!(
            differ.has_changed(&img),
            "force interval=0 should force capture on identical frame"
        );
    }

    #[test]
    fn multiple_consecutive_identical_frames() {
        let mut differ = FrameDiffer::new(FrameDiffConfig::default());
        let img = solid_image(100, 100, [100, 100, 100, 255]);

        assert!(differ.has_changed(&img)); // first = true
        assert!(!differ.has_changed(&img)); // same = false
        assert!(!differ.has_changed(&img)); // same = false
        assert!(!differ.has_changed(&img)); // same = false
    }

    #[test]
    fn change_after_static_period() {
        let mut differ = FrameDiffer::new(FrameDiffConfig::default());
        let gray = solid_image(100, 100, [128, 128, 128, 255]);
        let red = solid_image(100, 100, [255, 0, 0, 255]);

        assert!(differ.has_changed(&gray)); // first
        assert!(!differ.has_changed(&gray)); // no change
        assert!(!differ.has_changed(&gray)); // no change
        assert!(
            differ.has_changed(&red),
            "change after static should be detected"
        );
    }

    #[test]
    fn different_sized_images_handled() {
        let mut differ = FrameDiffer::new(FrameDiffConfig::default());
        let small = solid_image(50, 50, [0, 0, 0, 255]);
        let large = solid_image(200, 200, [255, 255, 255, 255]);

        assert!(differ.has_changed(&small)); // first
        assert!(
            differ.has_changed(&large),
            "different size + color should be a change"
        );
    }

    // --- downscale tests ---

    #[test]
    fn downscale_reduces_dimensions() {
        let img = solid_image(400, 300, [128, 128, 128, 255]);
        let small = downscale(&img, 4);
        assert_eq!(small.width(), 100);
        assert_eq!(small.height(), 75);
    }

    #[test]
    fn downscale_minimum_1x1() {
        let img = solid_image(2, 2, [128, 128, 128, 255]);
        let small = downscale(&img, 10);
        assert!(small.width() >= 1);
        assert!(small.height() >= 1);
    }

    // --- compare_histograms tests ---

    #[test]
    fn identical_histograms_zero_distance() {
        let img = solid_image(100, 100, [128, 128, 128, 255]);
        let distance = compare_histograms(&img, &img);
        assert!(
            distance < 0.001,
            "identical images should have ~0 distance, got {distance}"
        );
    }

    #[test]
    fn opposite_histograms_high_distance() {
        let black = solid_image(100, 100, [0, 0, 0, 255]);
        let white = solid_image(100, 100, [255, 255, 255, 255]);
        let distance = compare_histograms(&black, &white);
        assert!(
            distance > 0.5,
            "black vs white should have high distance, got {distance}"
        );
    }

    /// Create a gradient image (left=dark, right=bright) with a given offset.
    fn gradient_image(width: u32, height: u32, offset: u8) -> DynamicImage {
        let mut img = RgbaImage::new(width, height);
        for (x, _, pixel) in img.enumerate_pixels_mut() {
            #[allow(clippy::cast_possible_truncation)]
            let v = ((x * 255 / width) as u8).saturating_add(offset);
            *pixel = Rgba([v, v, v, 255]);
        }
        DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn similar_histograms_low_distance() {
        // Gradient images produce spread-out histograms that overlap well.
        let img1 = gradient_image(256, 100, 0);
        let img2 = gradient_image(256, 100, 2); // shifted by 2 values
        let distance = compare_histograms(&img1, &img2);
        assert!(
            distance < 0.15,
            "similar gradients should have low distance, got {distance}"
        );
    }

    #[test]
    fn histogram_is_symmetric() {
        let a = solid_image(100, 100, [50, 50, 50, 255]);
        let b = solid_image(100, 100, [200, 200, 200, 255]);
        let d1 = compare_histograms(&a, &b);
        let d2 = compare_histograms(&b, &a);
        assert!(
            (d1 - d2).abs() < 0.001,
            "histogram comparison should be symmetric: {d1} vs {d2}"
        );
    }

    // --- Custom threshold tests ---

    #[test]
    fn custom_zero_threshold_detects_any_change() {
        let mut differ = FrameDiffer::new(FrameDiffConfig {
            change_threshold: 0.0,
            ..FrameDiffConfig::default()
        });
        // Uniform images with different values: Hellinger distance = 1.0
        // (all mass at one bin vs another). With threshold=0, always detected.
        let gray1 = solid_image(100, 100, [128, 128, 128, 255]);
        let gray2 = solid_image(100, 100, [129, 129, 129, 255]);

        assert!(differ.has_changed(&gray1)); // first
        assert!(
            differ.has_changed(&gray2),
            "any change should be detected with threshold=0"
        );
    }
}
