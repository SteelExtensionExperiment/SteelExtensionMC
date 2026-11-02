//! Native entity snapshots, player-owned storage, and cancellable AI control.
//! Examples decide which entities to store, health policies, and gameplay rules.

use crate::events::EntityInfo;
use crate::players::Uuid;
use stabby::string::String as ApiString;

/// A world-local identity checked against both network ID and persistent UUID.
#[stabby::stabby]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityId {
    /// Current network ID.
    pub id: i32,
    /// Persistent entity UUID.
    pub uuid: Uuid,
}
impl EntityInfo {
    /// Copies the identity for later world operations; stale IDs are rejected.
    #[must_use]
    pub const fn identity(&self) -> EntityId {
        EntityId {
            id: self.id,
            uuid: self.uuid,
        }
    }
}

/// Current living-entity vitals.
#[stabby::stabby]
#[derive(Debug, Clone, Copy)]
pub struct Vitals {
    /// Current health.
    pub health: f32,
    /// Maximum health.
    pub maximum: f32,
}

/// A live entity restored from a player-owned snapshot.
#[stabby::stabby]
pub struct StoredEntity {
    /// Runtime storage activation identity; retained across native replacement conversions.
    pub instance: Uuid,
    /// Owner's compound path.
    pub path: ApiString,
    /// Current entity identity and position.
    pub entity: EntityInfo,
}

/// Explicit policies for live instances restored from external storage.
#[stabby::stabby]
#[derive(Debug, Clone, Copy)]
pub struct EntityPolicy {
    /// Lowest health allowed by health mutations; zero preserves native death behavior.
    /// A positive value also restores dead saved entities to this minimum on restoration.
    pub minimum_health: f32,
    /// Whether native death processing may produce loot and experience.
    pub drop_loot: bool,
}
impl Default for EntityPolicy {
    fn default() -> Self {
        Self {
            minimum_health: 0.0,
            drop_loot: true,
        }
    }
}

pub use control::EntityControl;
pub(crate) mod control;

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "Bindings consistently borrow server and world capability handles"
)]
#[stabby::host_api]
/// Store, restore, synchronize, return and release native entity snapshots.
pub mod api {
    use super::{EntityPolicy, StoredEntity};
    #[cfg(feature = "host")]
    use crate::host::entities as host;
    use crate::{
        ApiResult, PlayerId, Position, Server, World, events::EntityInfo, world_effects::EntityId,
    };
    #[cfg(feature = "host")]
    use stabby::string::String as ApiString;
    use stabby::{str::Str, vec::Vec as ApiVec};
    impl<'a> World<'a> {
        /// Stores a native serializable entity at a vacant path in the player's domain compound.
        /// Riders, leashes, players, unsupported factories and already-stored entities are rejected.
        pub fn store_entity(
            &self,
            player: PlayerId,
            target: EntityId,
            path: Str<'_>,
            policy: EntityPolicy,
        ) -> ApiResult<()> {
            host::store(self.host(), player, target, path.as_str(), policy)
                .map_err(ApiString::from)
                .into()
        }
    }
    impl<'a> Server<'a> {
        /// Restores saved native state at a prepared position using the caller's health policy.
        pub fn restore_entity(
            &self,
            player: PlayerId,
            path: Str<'_>,
            position: Position,
            policy: EntityPolicy,
        ) -> ApiResult<EntityInfo> {
            host::restore(self.host(), player, path.as_str(), position, policy)
                .map_err(ApiString::from)
                .into()
        }
        /// Synchronizes native health, state and deployment flags into player data.
        pub fn synchronize_stored_entities(&self, player: PlayerId) -> ApiResult<()> {
            host::synchronize(self.host(), player)
                .map_err(ApiString::from)
                .into()
        }
        /// Snapshots and removes one live instance, retaining its player data.
        pub fn return_entity(&self, player: PlayerId, path: Str<'_>) -> ApiResult<bool> {
            host::return_to_storage(self.host(), player, path.as_str())
                .map_err(ApiString::from)
                .into()
        }
        /// Releases a stored entity to ordinary world persistence and removes its data compound.
        /// An inactive entity needs a loaded, collision-free position; an active instance stays in place.
        pub fn release_entity(
            &self,
            player: PlayerId,
            path: Str<'_>,
            position: Position,
            policy: EntityPolicy,
        ) -> ApiResult<EntityInfo> {
            host::release(self.host(), player, path.as_str(), position, policy)
                .map_err(ApiString::from)
                .into()
        }
        /// Current deployments; local IDs are paired with persistent UUIDs.
        pub fn stored_entity_instances(&self, player: PlayerId) -> ApiResult<ApiVec<StoredEntity>> {
            host::instances(self.host(), player)
                .map_err(ApiString::from)
                .into()
        }
    }
}
