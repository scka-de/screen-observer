use core_graphics::access::ScreenCaptureAccess;

/// Screen recording permission status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    /// Permission has been granted.
    Granted,
    /// Permission has been denied or not yet requested.
    Denied,
}

/// Check if Screen Recording permission is granted (without prompting).
#[must_use]
pub fn check_screen_recording() -> PermissionStatus {
    let access = ScreenCaptureAccess;
    if access.preflight() {
        PermissionStatus::Granted
    } else {
        PermissionStatus::Denied
    }
}

/// Request Screen Recording permission (shows system dialog if not yet asked).
///
/// Returns the resulting permission status. Note that on macOS,
/// the user must restart the app after granting permission.
#[must_use]
pub fn request_screen_recording() -> PermissionStatus {
    let access = ScreenCaptureAccess;
    if access.request() {
        PermissionStatus::Granted
    } else {
        PermissionStatus::Denied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_status_equality() {
        assert_eq!(PermissionStatus::Granted, PermissionStatus::Granted);
        assert_ne!(PermissionStatus::Granted, PermissionStatus::Denied);
    }

    #[test]
    fn check_does_not_panic() {
        // Just verify it runs — result depends on system state.
        let _ = check_screen_recording();
    }
}
