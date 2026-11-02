//! Gameplay hooks for mobs registered with `ContentRegistry::register_entity`, and
//! spawning them.
//!
//! The host runs the vanilla behavior of the mob's style; these hooks add what makes
//! it distinct. Every hook runs on the tick thread of the world it receives.

use crate::World;
use crate::block_behavior::types::Entity;

/// Optional mob hooks. Unimplemented hooks do nothing.
#[stabby::stabby(checked)]
pub trait EntityBehavior: Send + Sync {
    /// Runs at the end of each of the mob's AI ticks.
    extern "C" fn tick<'a>(&self, world: World<'a>, mob: Entity<'a>) {
        let _ = (world, mob);
    }

    /// The mob hurt `target` with a melee or contact attack.
    extern "C" fn attacked<'a>(&self, world: World<'a>, mob: Entity<'a>, target: Entity<'a>) {
        let _ = (world, mob, target);
    }

    /// The mob was killed; it is still in the world at its final position.
    extern "C" fn died<'a>(&self, world: World<'a>, mob: Entity<'a>) {
        let _ = (world, mob);
    }
}

/// Owning, ABI-checked mob hooks. The extension's destructor frees its own data.
pub type DynEntityBehavior = stabby::dynptr!(stabby::boxed::Box<dyn EntityBehavior + Send + Sync>);

#[stabby::host_api]
/// Mob hook registration and spawning.
pub mod api {
    use super::DynEntityBehavior;
    #[cfg(feature = "host")]
    use crate::host::entity::{self as host_entity, EntityRegistration};
    use crate::players::Uuid;
    use crate::{ApiResult, Position};
    use stabby::str::Str;
    #[cfg(feature = "host")]
    use stabby::string::String as ApiString;

    /// Exclusive access to mob hooks before they are published.
    #[opaque]
    pub struct EntityBehaviors<'a>(&'a mut EntityRegistration<'a>);

    impl<'a> EntityBehaviors<'a> {
        /// This extension's TOML configuration.
        #[must_use]
        pub fn config(&self) -> stabby::string::String {
            self.host().config.into()
        }

        /// Attaches hooks to a mob type registered by `register_entity`. A type accepts
        /// hooks once.
        pub fn set_entity_behavior(
            &mut self,
            entity_type: Str<'_>,
            behavior: DynEntityBehavior,
        ) -> ApiResult<()> {
            host_entity::set_entity_behavior(self.host_mut(), entity_type.as_str(), behavior)
                .map_err(ApiString::from)
                .into()
        }
    }

    impl<'a> crate::World<'a> {
        /// Spawns a mob of any summonable type (vanilla or registered) at `position`,
        /// returning its UUID. The chunk must be loaded.
        pub fn spawn_entity(&self, entity_type: Str<'_>, position: Position) -> ApiResult<Uuid> {
            host_entity::spawn_entity(self.host(), entity_type.as_str(), position)
                .map_err(ApiString::from)
                .into()
        }

        /// Counts live entities of a type within `radius` blocks of `center`, in loaded chunks.
        pub fn count_entities(
            &self,
            entity_type: Str<'_>,
            center: Position,
            radius: f64,
        ) -> ApiResult<u32> {
            host_entity::count_entities(self.host(), entity_type.as_str(), center, radius)
                .map_err(ApiString::from)
                .into()
        }
    }

    impl<'a> crate::Hooks<'a> {
        /// Subscribes during pre-init. Runs while entity factories initialize, with
        /// every registered mob type available.
        pub fn register_entity_behaviors(
            &mut self,
            callback: extern "C" fn(EntityBehaviors<'_>) -> ApiResult<()>,
        ) -> ApiResult<()> {
            host_entity::subscribe(self.host_mut(), callback)
                .map_err(ApiString::from)
                .into()
        }
    }
}
