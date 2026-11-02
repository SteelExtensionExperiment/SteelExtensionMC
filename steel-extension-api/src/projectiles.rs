//! Custom item projectiles with native movement and collision on vanilla clients.
//! Impacts run on the world's normal tick thread. Launching does not consume inventory.

/// Motion and lifetime of a custom projectile.
#[stabby::stabby]
#[derive(Debug, Clone, Copy)]
pub struct ProjectileSpec {
    /// Initial speed, in blocks per tick.
    pub speed: f32,
    /// Per-axis native launch uncertainty.
    pub uncertainty: f32,
    /// Downward acceleration per tick.
    /// Zero syncs vanilla `NoGravity`; the snowball client otherwise predicts its default gravity.
    pub gravity: f64,
    /// Maximum lifetime in world ticks.
    pub lifetime: u32,
}

pub use api::Impact;

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "Borrowed world handles follow the shared receiver convention of the World API"
)]
#[stabby::host_api]
/// Launch operations and synchronous impact access.
pub mod api {
    use super::ProjectileSpec;
    #[cfg(feature = "host")]
    use crate::host::events::{block_pos, entity_info, face};
    #[cfg(feature = "host")]
    use crate::host::items;
    #[cfg(feature = "host")]
    use crate::host::projectiles::{self as host, ImpactAccess};
    use crate::{
        ApiResult, BlockPos, PlayerId, Position, Server, World,
        events::{EntityInfo, Face},
        items::ItemSpec,
    };
    #[cfg(feature = "host")]
    use stabby::string::String as ApiString;
    use stabby::{option::Option as ApiOption, str::Str};
    #[cfg(feature = "host")]
    use steel_core::entity::{Projectile, ProjectileHit, ThrowableItemProjectile};
    #[cfg(feature = "host")]
    use steel_core::extension::events::EntityInfo as NativeEntityInfo;

    /// Valid only during an impact callback; retain copied identities or player data instead.
    #[opaque]
    pub struct Impact<'a>(&'a ImpactAccess<'a>);

    impl<'a> World<'a> {
        /// Throws an item from this player's eyes, using the player's current aim.
        /// The callback runs once on impact, without locks held across the native boundary.
        pub fn throw_projectile(
            &self,
            player: PlayerId,
            item: ItemSpec,
            spec: ProjectileSpec,
            callback: extern "C" fn(Server<'_>, Impact<'_>, u64),
            token: u64,
        ) -> ApiResult<i32> {
            host::launch(self.host(), player, &item, spec, None, callback, token)
                .map_err(ApiString::from)
                .into()
        }
        /// Throws toward a known hit point, retaining native shooter movement and collision.
        pub fn throw_projectile_towards(
            &self,
            player: PlayerId,
            target: Position,
            item: ItemSpec,
            spec: ProjectileSpec,
            callback: extern "C" fn(Server<'_>, Impact<'_>, u64),
            token: u64,
        ) -> ApiResult<i32> {
            host::launch(
                self.host(),
                player,
                &item,
                spec,
                Some(target),
                callback,
                token,
            )
            .map_err(ApiString::from)
            .into()
        }
    }
    impl<'a> Impact<'a> {
        /// Rendered projectile item, including the data attached when it was launched.
        #[must_use]
        pub fn item(&self) -> ItemSpec {
            items::to_spec(&self.host().projectile.get_item())
        }
        /// Applies native projectile damage with this exact projectile and shooter as sources.
        pub fn hurt_target(&self, cause: Str<'_>, amount: f32) -> ApiResult<bool> {
            host::hurt_target(self.host(), cause.as_str(), amount)
                .map_err(ApiString::from)
                .into()
        }
        /// Hit block position, or `None` for an entity collision.
        #[must_use]
        pub fn block(&self) -> ApiOption<BlockPos> {
            match self.host().hit {
                ProjectileHit::Block { hit, .. } => Some(block_pos(hit.block_pos)),
                ProjectileHit::Entity(_) => None,
            }
            .into()
        }
        /// Hit block face, or `None` for an entity collision.
        #[must_use]
        pub fn face(&self) -> ApiOption<Face> {
            match self.host().hit {
                ProjectileHit::Block { hit, .. } => Some(face(hit.direction)),
                ProjectileHit::Entity(_) => None,
            }
            .into()
        }
        /// The world containing this impact.
        #[must_use]
        pub fn world(&self) -> World<'a> {
            World::from_host(self.host().world)
        }
        /// Exact world-space impact point.
        #[must_use]
        pub fn position(&self) -> Position {
            let pos = self.host().hit.location();
            Position {
                x: pos.x,
                y: pos.y,
                z: pos.z,
            }
        }
        /// Hit entity, or `None` for a block collision.
        #[must_use]
        pub fn target(&self) -> ApiOption<EntityInfo> {
            match self.host().hit {
                ProjectileHit::Entity(hit) => {
                    Some(entity_info(&NativeEntityInfo::of(hit.entity.as_ref())))
                }
                ProjectileHit::Block { .. } => None,
            }
            .into()
        }
        /// The shooter, if still present in the same world.
        #[must_use]
        pub fn owner(&self) -> ApiOption<EntityInfo> {
            self.host()
                .projectile
                .get_owner()
                .map(|entity| entity_info(&NativeEntityInfo::of(entity.as_ref())))
                .into()
        }
    }
}
