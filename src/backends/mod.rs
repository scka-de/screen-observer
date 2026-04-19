// Pluggable backends for screen observation.

pub mod mock;
#[cfg(all(target_os = "macos", feature = "native"))]
pub mod native;
pub mod screenpipe;
