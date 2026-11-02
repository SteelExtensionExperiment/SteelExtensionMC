use glam::DVec3;

/// Direct flight steering for mobs whose native AI does not use pathfinding.
/// Implementations retain their native acceleration, rotation and travel input.
pub trait FlightControl: Send + Sync {
    /// Wakes the mob, if needed, and steers toward a point using native flight rules.
    fn fly_towards(&self, target: DVec3);
}
