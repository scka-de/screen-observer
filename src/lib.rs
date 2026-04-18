#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod backends;
pub mod types;

use anyhow::Result;
use tokio::sync::broadcast;

// Re-export key types at crate root for convenience.
pub use types::{EventType, ObservationEvent, WindowContext};

/// Trait that all screen observation backends must implement.
///
/// Backends produce `ObservationEvent`s via a broadcast channel.
/// Multiple subscribers can listen to the same observer.
#[allow(async_fn_in_trait)]
pub trait ScreenObserver: Send {
    /// Start observing the screen. Events begin flowing to subscribers.
    async fn start(&mut self) -> Result<()>;

    /// Stop observing. No more events are emitted after this returns.
    async fn stop(&mut self) -> Result<()>;

    /// Get a receiver for observation events.
    /// Can be called multiple times to create multiple subscribers.
    fn subscribe(&self) -> broadcast::Receiver<ObservationEvent>;

    /// Whether the observer is currently running.
    fn is_running(&self) -> bool;
}
