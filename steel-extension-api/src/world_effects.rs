//! Gameplay effects on existing entities, independent of projectile and terrain lifetimes.

pub use crate::entities::{EntityId, Vitals};

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "World bindings consistently borrow their capability handle"
)]
#[stabby::host_api]
/// Entity effects and registry-backed damage classification.
pub mod api {
    use super::{EntityId, Vitals};
    #[cfg(feature = "host")]
    use crate::host::world_effects as host;
    use crate::{
        ApiResult, Position, World,
        effects::Effect,
        events::{DamageEvent, EntityInfo},
    };
    #[cfg(feature = "host")]
    use stabby::string::String as ApiString;
    use stabby::{option::Option as ApiOption, str::Str, vec::Vec as ApiVec};
    impl<'a> World<'a> {
        /// Living, nonremoved entities whose bounding boxes intersect a radius-sized cube.
        pub fn entities_near(
            &self,
            position: Position,
            radius: f64,
        ) -> ApiResult<ApiVec<EntityInfo>> {
            host::near(self.host(), position, radius)
                .map_err(ApiString::from)
                .into()
        }
        /// Reads current health; players and mobs both use their native living capability.
        pub fn entity_vitals(&self, id: EntityId) -> ApiResult<Vitals> {
            host::vitals(self.host(), id)
                .map_err(ApiString::from)
                .into()
        }
        /// Deals registry-backed native damage, including armor, events and death.
        pub fn hurt_entity(
            &self,
            id: EntityId,
            cause: Str<'_>,
            amount: f32,
            owner: ApiOption<EntityId>,
        ) -> ApiResult<bool> {
            host::hurt(self.host(), id, cause.as_str(), amount, owner.into())
                .map_err(ApiString::from)
                .into()
        }
        /// Applies a native mob effect to an existing living entity.
        pub fn affect_entity(&self, id: EntityId, effect: Effect) -> ApiResult<bool> {
            host::affect(self.host(), id, &effect)
                .map_err(ApiString::from)
                .into()
        }
        /// Ignites an entity through the native fire timer and fire immunity rules.
        pub fn ignite_entity(&self, id: EntityId, ticks: i32) -> ApiResult<()> {
            host::ignite(self.host(), id, ticks)
                .map_err(ApiString::from)
                .into()
        }
    }
    impl<'a> DamageEvent<'a> {
        /// Tests the actual damage type against a registered tag, such as `minecraft:is_projectile`.
        pub fn cause_in_tag(&self, tag: Str<'_>) -> ApiResult<bool> {
            host::cause_in_tag(&self.host().event.cause, tag.as_str())
                .map_err(ApiString::from)
                .into()
        }
    }
}
