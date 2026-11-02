//! Optional policies supplied by the owner of a live entity attachment.

use super::{EntityBase, SharedEntity};
use std::sync::Arc;

/// An attachment may own persistence and follow replacement conversions.
/// Entities retain a weak reference so an attachment can hold its live entity.
pub trait EntityAttachment: Send + Sync {
    /// Whether ordinary chunk saves should include this entity.
    fn save_in_chunk(&self) -> bool;
    /// Whether this attachment can follow an entity through a portal.
    fn can_use_portal(&self) -> bool;
    /// Whether native death processing should produce loot and experience.
    fn drop_loot(&self) -> bool;
    /// Adjusts requested health before the native zero-to-maximum clamp.
    fn health(&self, requested: f32) -> f32;
    /// Checks a replacement before it is admitted to the world.
    fn can_convert(&self, destination: &SharedEntity) -> bool;
    /// Transfers to an admitted replacement, or rejects a stale conversion.
    fn replace_converted(self: Arc<Self>, from: &EntityBase, to: &SharedEntity) -> bool;
}
