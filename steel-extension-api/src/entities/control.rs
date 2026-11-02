pub use api::EntityControl;

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "World and callback accessors consistently borrow capability handles"
)]
#[stabby::host_api]
/// Native AI capabilities, independent of any particular entity game or extension.
pub mod api {
    #[cfg(feature = "host")]
    use crate::host::entities::control::{self as host, ControlAccess};
    use crate::{ApiResult, Position, World, events::EntityInfo, world_effects::EntityId};
    use stabby::str::Str;
    #[cfg(feature = "host")]
    use stabby::string::String as ApiString;

    /// Borrowed access to the entity currently executing an AI callback.
    #[opaque]
    pub struct EntityControl<'a>(&'a ControlAccess<'a>);

    impl<'a> World<'a> {
        /// Resolves both parts of an identity; stale IDs are rejected.
        pub fn entity_info(&self, entity: EntityId) -> ApiResult<EntityInfo> {
            host::info(self.host(), entity)
                .map_err(ApiString::from)
                .into()
        }
        /// Whether this entity implements native mob AI, including ambient mobs.
        pub fn entity_has_ai(&self, entity: EntityId) -> ApiResult<bool> {
            host::has_ai(self.host(), entity)
                .map_err(ApiString::from)
                .into()
        }
        /// Rejects replacement of an unfinished controller.
        /// Callbacks execute during native AI steps; returning false retires the controller.
        /// Native sensing, navigation, movement, look, jump, collision and damage remain active.
        pub fn control_entity(
            &self,
            entity: EntityId,
            callback: extern "C" fn(EntityControl<'_>, u64) -> bool,
            token: u64,
        ) -> ApiResult<()> {
            host::install(self.host(), entity, callback, token)
                .map_err(ApiString::from)
                .into()
        }
        /// Whether an entity currently has an unfinished custom AI controller.
        pub fn entity_is_controlled(&self, entity: EntityId) -> ApiResult<bool> {
            host::is_controlled(self.host(), entity)
                .map_err(ApiString::from)
                .into()
        }
    }

    impl<'a> EntityControl<'a> {
        /// Current world; the handle is valid only for this callback.
        #[must_use]
        pub fn world(&self) -> World<'a> {
            World::from_host(self.host().world)
        }
        /// Identity and position of the controlled entity.
        #[must_use]
        pub fn entity(&self) -> EntityInfo {
            host::current(self.host())
        }
        /// Whether this mob supports navigation or direct flight.
        #[must_use]
        pub fn can_move(&self) -> bool {
            host::can_move(self.host())
        }
        /// Looks toward and approaches an entity using its native movement capability.
        /// `repath` requests a new ground path; existing paths continue ticking between requests.
        pub fn move_to_entity(&self, target: EntityId, speed: f64, repath: bool) -> ApiResult<()> {
            host::move_to(self.host(), target, speed, repath)
                .map_err(ApiString::from)
                .into()
        }
        /// Changes native look control without changing movement.
        pub fn look_at(
            &self,
            position: Position,
            yaw_limit: f32,
            pitch_limit: f32,
        ) -> ApiResult<()> {
            host::look_at(self.host(), position, yaw_limit, pitch_limit)
                .map_err(ApiString::from)
                .into()
        }
        /// Deals native damage with the controlled entity as both direct source and attacker.
        pub fn hurt_target(
            &self,
            target: EntityId,
            cause: Str<'_>,
            amount: f32,
        ) -> ApiResult<bool> {
            host::hurt_target(self.host(), target, cause.as_str(), amount)
                .map_err(ApiString::from)
                .into()
        }
        /// Native melee reach and line of sight, without causing damage.
        pub fn can_melee_attack(&self, target: EntityId) -> ApiResult<bool> {
            host::can_melee_attack(self.host(), target)
                .map_err(ApiString::from)
                .into()
        }
    }
}
