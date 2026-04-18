#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod backends;
pub mod types;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::broadcast;

// Re-export key types at crate root for convenience.
pub use types::{EventType, ObservationEvent, WindowContext};

/// Trait that all screen observation backends must implement.
///
/// Backends produce `ObservationEvent`s via a broadcast channel.
/// Multiple subscribers can listen to the same observer.
///
/// # Broadcast semantics
///
/// `subscribe()` returns a `broadcast::Receiver`. If a subscriber falls behind
/// by more than the channel capacity, it will receive `RecvError::Lagged(n)`
/// indicating how many events were dropped. Consumers should handle this
/// gracefully — lost observations are acceptable in the commentary pipeline.
#[async_trait]
pub trait ScreenObserver: Send {
    /// Start observing the screen. Events begin flowing to subscribers.
    async fn start(&mut self) -> Result<()>;

    /// Stop observing. No more events are emitted after this returns.
    async fn stop(&mut self) -> Result<()>;

    /// Get a receiver for observation events.
    ///
    /// Can be called multiple times to create multiple subscribers.
    /// Each subscriber independently tracks its position in the channel.
    ///
    /// If a subscriber is slow and the channel buffer fills, older events
    /// are dropped and the next `recv()` returns `RecvError::Lagged(n)`.
    fn subscribe(&self) -> broadcast::Receiver<ObservationEvent>;

    /// Whether the observer is currently running.
    fn is_running(&self) -> bool;
}
