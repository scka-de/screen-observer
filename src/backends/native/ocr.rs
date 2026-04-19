// Link Vision and CoreImage frameworks so their classes are available at runtime.
#[link(name = "Vision", kind = "framework")]
extern "C" {}

#[link(name = "CoreImage", kind = "framework")]
extern "C" {}

use std::ffi::c_void;

use anyhow::{Context, Result};
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::image::CGImage;
use foreign_types_shared::ForeignType;
use image::DynamicImage;
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::Id;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{class, msg_send, msg_send_id};

/// Wrapper for `CGImageRef` that implements objc2's `Encode` trait.
#[repr(transparent)]
struct CGImagePtr(*const c_void);

unsafe impl Encode for CGImagePtr {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[]));
}

unsafe impl RefEncode for CGImagePtr {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[]));
}

/// Result of OCR text recognition.
#[derive(Debug, Clone)]
pub struct OcrResult {
    /// Extracted text (concatenated from all recognized observations).
    pub text: String,
    /// Average confidence across all recognized text (0.0 to 1.0).
    pub confidence: f64,
}

/// Recognize text in an image using Apple Vision framework.
///
/// Runs `VNRecognizeTextRequest` on a `CGImage` created from the input.
/// This is a **blocking** call — wrap in `tokio::task::spawn_blocking`.
///
/// # Errors
///
/// Returns an error if the Vision request fails or `CGImage` creation fails.
pub fn recognize_text(image: &DynamicImage) -> Result<OcrResult> {
    let rgba = image.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    let raw = rgba.into_raw();

    let cg_image = create_cg_image(&raw, width, height)?;

    objc2::rc::autoreleasepool(|_| unsafe { run_vision_ocr(&cg_image) })
}

/// Create a `CGImage` from raw RGBA pixel data via a `CGContext`.
fn create_cg_image(data: &[u8], width: u32, height: u32) -> Result<CGImage> {
    let color_space = CGColorSpace::create_device_rgb();
    let bytes_per_row = width as usize * 4;

    // CGContext needs a mutable pointer even for read-only operations.
    // The data is only read during create_image().
    let data_ptr = data.as_ptr().cast_mut().cast::<std::ffi::c_void>();
    let ctx = CGContext::create_bitmap_context(
        Some(data_ptr),
        width as usize,
        height as usize,
        8,
        bytes_per_row,
        &color_space,
        // kCGImageAlphaNoneSkipLast — Vision OCR doesn't need alpha channel.
        core_graphics::base::kCGImageAlphaNoneSkipLast,
    );

    ctx.create_image()
        .context("failed to create CGImage from bitmap context")
}

/// Run Apple Vision text recognition on a `CGImage`.
///
/// # Safety
///
/// Uses Objective-C runtime message passing. Caller must ensure
/// this runs in a thread with an autorelease pool.
#[allow(clippy::cast_possible_truncation)]
unsafe fn run_vision_ocr(cg_image: &CGImage) -> Result<OcrResult> {
    // VNImageRequestHandler *handler = [[VNImageRequestHandler alloc]
    //   initWithCGImage:cgImage options:@{}];
    let handler_cls = class!(VNImageRequestHandler);
    let options: Id<NSObject> = msg_send_id![class!(NSDictionary), dictionary];

    // Wrap CGImage pointer so objc2 encodes it as ^{CGImage=}.
    let cg_ptr = CGImagePtr(cg_image.as_ptr().cast());
    let handler: Id<NSObject> = msg_send_id![
        msg_send_id![handler_cls, alloc],
        initWithCGImage: cg_ptr,
        options: &*options
    ];

    // VNRecognizeTextRequest *request = [[VNRecognizeTextRequest alloc] init];
    let request: Id<NSObject> =
        msg_send_id![msg_send_id![class!(VNRecognizeTextRequest), alloc], init];

    // [request setRecognitionLevel:0]; // 0 = VNRequestTextRecognitionLevelAccurate
    let _: () = msg_send![&*request, setRecognitionLevel: 0_isize];

    // Build requests array.
    let requests: Id<NSObject> = msg_send_id![
        class!(NSArray),
        arrayWithObject: &*request
    ];

    // [handler performRequests:requests error:&error];
    let mut error: *mut AnyObject = std::ptr::null_mut();
    let success: bool = msg_send![
        &*handler,
        performRequests: &*requests,
        error: std::ptr::addr_of_mut!(error)
    ];

    if !success {
        if !error.is_null() {
            let desc: Id<NSObject> = msg_send_id![error, localizedDescription];
            let c_str: *const std::ffi::c_char = msg_send![&*desc, UTF8String];
            if !c_str.is_null() {
                let msg = std::ffi::CStr::from_ptr(c_str).to_string_lossy();
                anyhow::bail!("Vision OCR failed: {msg}");
            }
        }
        anyhow::bail!("Vision OCR failed with unknown error");
    }

    // NSArray *results = [request results];
    let results: *mut AnyObject = msg_send![&*request, results];
    if results.is_null() {
        return Ok(OcrResult {
            text: String::new(),
            confidence: 0.0,
        });
    }

    let count: usize = msg_send![results, count];

    let mut full_text = String::new();
    let mut total_confidence: f64 = 0.0;
    let mut text_count: usize = 0;

    for i in 0..count {
        let observation: *mut AnyObject = msg_send![results, objectAtIndex: i];
        if observation.is_null() {
            continue;
        }

        // NSArray *candidates = [observation topCandidates:1];
        let candidates: *mut AnyObject = msg_send![observation, topCandidates: 1_usize];
        if candidates.is_null() {
            continue;
        }

        let cand_count: usize = msg_send![candidates, count];
        if cand_count == 0 {
            continue;
        }

        let candidate: *mut AnyObject = msg_send![candidates, objectAtIndex: 0_usize];
        if candidate.is_null() {
            continue;
        }

        // NSString *text = [candidate string];
        let ns_string: *mut AnyObject = msg_send![candidate, string];
        if ns_string.is_null() {
            continue;
        }

        let c_str: *const std::ffi::c_char = msg_send![ns_string, UTF8String];
        if c_str.is_null() {
            continue;
        }
        let text = std::ffi::CStr::from_ptr(c_str)
            .to_string_lossy()
            .to_string();

        // float confidence = [candidate confidence];
        let confidence: f32 = msg_send![candidate, confidence];

        if !text.is_empty() {
            if !full_text.is_empty() {
                full_text.push('\n');
            }
            full_text.push_str(&text);
            total_confidence += f64::from(confidence);
            text_count += 1;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_confidence = if text_count > 0 {
        total_confidence / text_count as f64
    } else {
        0.0
    };

    Ok(OcrResult {
        text: full_text,
        confidence: avg_confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn blank_image(width: u32, height: u32) -> DynamicImage {
        let img = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));
        DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn ocr_result_default_values() {
        let result = OcrResult {
            text: String::new(),
            confidence: 0.0,
        };
        assert!(result.text.is_empty());
        assert!((result.confidence - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn ocr_result_clone() {
        let result = OcrResult {
            text: "Hello".to_string(),
            confidence: 0.9,
        };
        let cloned = result.clone();
        assert_eq!(cloned.text, "Hello");
        assert!((cloned.confidence - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn create_cg_image_small() {
        let data: Vec<u8> = vec![255, 0, 0, 255].repeat(4);
        let result = create_cg_image(&data, 2, 2);
        assert!(result.is_ok(), "CGImage creation should succeed");
    }

    #[test]
    fn create_cg_image_from_dynamic() {
        let img = blank_image(100, 100);
        let rgba = img.to_rgba8();
        let raw = rgba.as_raw();
        let result = create_cg_image(raw, 100, 100);
        assert!(result.is_ok());
    }

    #[test]
    fn create_cg_image_large() {
        let img = blank_image(1920, 1080);
        let rgba = img.to_rgba8();
        let raw = rgba.as_raw();
        let result = create_cg_image(raw, 1920, 1080);
        assert!(result.is_ok());
    }

    #[test]
    #[ignore = "requires macOS Vision framework"]
    fn recognize_blank_image_returns_empty() {
        let img = blank_image(200, 200);
        let result = recognize_text(&img).unwrap();
        assert!(
            result.text.is_empty(),
            "blank image should produce no text, got: '{}'",
            result.text
        );
    }

    #[test]
    #[ignore = "requires macOS Vision framework"]
    fn recognize_text_does_not_panic_on_small_image() {
        let img = blank_image(10, 10);
        let _ = recognize_text(&img);
    }

    #[test]
    #[ignore = "requires macOS Vision framework"]
    fn recognize_text_confidence_in_range() {
        let img = blank_image(200, 200);
        if let Ok(result) = recognize_text(&img) {
            assert!(
                result.confidence >= 0.0 && result.confidence <= 1.0,
                "confidence out of range: {}",
                result.confidence
            );
        }
    }

    #[test]
    #[ignore = "requires macOS Vision framework + Screen Recording"]
    fn recognize_text_from_screenshot() {
        if let Ok(img) = super::super::capture::capture_screen() {
            let result = recognize_text(&img);
            assert!(result.is_ok(), "OCR should not error on real screenshot");
        }
    }
}
