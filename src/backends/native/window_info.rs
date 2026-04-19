use std::ffi::c_void;

use core_foundation::base::{CFTypeID, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::display::{
    kCGNullWindowID, kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly,
    CGWindowListCopyWindowInfo,
};

use crate::types::WindowContext;

/// RAII guard that releases a `CFArray` on drop.
struct CfArrayGuard(*const core_foundation::array::__CFArray);
impl Drop for CfArrayGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { core_foundation::base::CFRelease(self.0.cast()) }
        }
    }
}

/// Information about the frontmost window.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub app_name: String,
    pub window_title: Option<String>,
    pub owner_pid: i32,
    pub is_on_screen: bool,
}

/// Get info about the frontmost (focused) normal window.
///
/// Returns `None` if no windows are found or `CGWindowList` fails.
#[must_use]
pub fn get_focused_window() -> Option<WindowInfo> {
    let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let window_list = unsafe { CGWindowListCopyWindowInfo(options, kCGNullWindowID) };

    if window_list.is_null() {
        return None;
    }

    // Guard ensures CFRelease runs exactly once on all paths.
    let _guard = CfArrayGuard(window_list);

    let count = unsafe { core_foundation::array::CFArrayGetCount(window_list) };

    // Walk z-order (front to back) and find the first normal window.
    for i in 0..count {
        let dict: CFDictionaryRef =
            unsafe { core_foundation::array::CFArrayGetValueAtIndex(window_list, i).cast() };

        let layer = get_number(dict, "kCGWindowLayer").unwrap_or(-1);
        if layer != 0 {
            continue; // Skip menu bar, overlays, etc.
        }

        let app_name = match get_string(dict, "kCGWindowOwnerName") {
            Some(name) if !name.is_empty() => name,
            _ => continue,
        };

        // Window bounds are nested under kCGWindowBounds as a sub-dictionary.
        if let Some(bounds) = get_sub_dict(dict, "kCGWindowBounds") {
            let width = get_number(bounds, "Width").unwrap_or(0);
            let height = get_number(bounds, "Height").unwrap_or(0);
            if width < 100 || height < 100 {
                continue; // Skip tiny windows (tooltips, decorations).
            }
        }

        let window_title = get_string(dict, "kCGWindowName");
        let owner_pid = get_number(dict, "kCGWindowOwnerPID").unwrap_or(0);
        let is_on_screen = get_bool(dict, "kCGWindowIsOnscreen").unwrap_or(false);

        return Some(WindowInfo {
            app_name,
            window_title,
            owner_pid: i32::try_from(owner_pid).unwrap_or(0),
            is_on_screen,
        });
    }

    None
}

/// Convert a `WindowInfo` into a `WindowContext` for event emission.
impl From<&WindowInfo> for WindowContext {
    fn from(info: &WindowInfo) -> Self {
        Self {
            app_bundle_id: None,
            app_name: info.app_name.clone(),
            window_title: info.window_title.clone(),
            browser_url: None,
            bounding_box: None,
        }
    }
}

// --- CFDictionary value extraction helpers ---
// Each helper reads a value from a CFDictionary without retaining it.
// Values are borrowed from the dictionary — no CFRelease needed.

fn get_cf_value(dict: CFDictionaryRef, key: &str) -> Option<(*const c_void, CFTypeID)> {
    unsafe {
        let cf_key = CFString::new(key);
        let mut value: *const c_void = std::ptr::null();
        if core_foundation::dictionary::CFDictionaryGetValueIfPresent(
            dict,
            cf_key.as_concrete_TypeRef().cast(),
            std::ptr::addr_of_mut!(value),
        ) != 0
            && !value.is_null()
        {
            let type_id = core_foundation::base::CFGetTypeID(value.cast());
            Some((value, type_id))
        } else {
            None
        }
    }
}

fn get_string(dict: CFDictionaryRef, key: &str) -> Option<String> {
    let (value, type_id) = get_cf_value(dict, key)?;
    if type_id == CFString::type_id() {
        // CFString from Get rule — not retained, so we copy immediately.
        let cf_str = unsafe { CFString::wrap_under_get_rule(value.cast()) };
        let result = cf_str.to_string();
        std::mem::forget(cf_str); // Don't release — we don't own it.
        Some(result)
    } else {
        None
    }
}

fn get_number(dict: CFDictionaryRef, key: &str) -> Option<i64> {
    let (value, type_id) = get_cf_value(dict, key)?;
    if type_id == CFNumber::type_id() {
        let cf_num = unsafe { CFNumber::wrap_under_get_rule(value.cast()) };
        let result = cf_num.to_i64();
        std::mem::forget(cf_num); // Don't release — we don't own it.
        result
    } else {
        None
    }
}

fn get_bool(dict: CFDictionaryRef, key: &str) -> Option<bool> {
    let (value, type_id) = get_cf_value(dict, key)?;
    if type_id == CFBoolean::type_id() {
        let cf_bool = unsafe { CFBoolean::wrap_under_get_rule(value.cast()) };
        let result = cf_bool == CFBoolean::true_value();
        std::mem::forget(cf_bool); // Don't release — we don't own it.
        Some(result)
    } else {
        None
    }
}

fn get_sub_dict(dict: CFDictionaryRef, key: &str) -> Option<CFDictionaryRef> {
    let (value, type_id) = get_cf_value(dict, key)?;
    let dict_type_id = unsafe { core_foundation::dictionary::CFDictionaryGetTypeID() };
    if type_id == dict_type_id {
        Some(value.cast())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_info_to_context() {
        let info = WindowInfo {
            app_name: "Safari".to_string(),
            window_title: Some("Google".to_string()),
            owner_pid: 12345,
            is_on_screen: true,
        };
        let ctx = WindowContext::from(&info);
        assert_eq!(ctx.app_name, "Safari");
        assert_eq!(ctx.window_title, Some("Google".to_string()));
        assert!(ctx.app_bundle_id.is_none());
    }

    #[test]
    #[ignore = "requires macOS with windows open"]
    fn get_focused_window_returns_something() {
        let info = get_focused_window();
        assert!(info.is_some(), "expected at least one window");
        let w = info.unwrap();
        assert!(!w.app_name.is_empty());
    }
}
