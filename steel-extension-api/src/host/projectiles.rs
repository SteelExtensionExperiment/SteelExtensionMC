//! Native projectile adapters; extension libraries remain pinned for the server lifetime.

mod entity;
use entity::{ExtensionProjectile, ImpactHandler};

use crate::{
    PlayerId, Position, Server,
    items::ItemSpec,
    projectiles::{Impact, ProjectileSpec},
};
use std::sync::{Arc, Weak};
use steel_core::server::Server as CoreServer;
use steel_core::{
    entity::{
        Entity, EntityAnchor, Projectile, ProjectileHit, SharedEntity, ThrowableItemProjectile,
        next_entity_id,
    },
    world::World as CoreWorld,
};

pub(crate) struct ImpactAccess<'a> {
    pub world: &'a Arc<CoreWorld>,
    pub projectile: &'a Arc<ExtensionProjectile>,
    pub hit: &'a ProjectileHit,
}

pub(crate) fn hurt_target(
    access: &ImpactAccess<'_>,
    cause: &str,
    amount: f32,
) -> Result<bool, String> {
    use steel_core::entity::damage::DamageSource;
    use steel_registry::{REGISTRY, RegistryExt};
    if !amount.is_finite() || amount < 0.0 {
        return Err("damage must be finite and nonnegative".into());
    }
    let key: steel_utils::Identifier = cause.parse().map_err(str::to_owned)?;
    let kind = REGISTRY
        .damage_types
        .by_key(&key)
        .ok_or("unknown damage type")?;
    let ProjectileHit::Entity(hit) = access.hit else {
        return Ok(false);
    };
    let direct: SharedEntity = Arc::clone(access.projectile) as SharedEntity;
    let mut source = DamageSource::environment(kind).with_direct_entity(direct);
    if let Some(owner) = access.projectile.get_owner() {
        source = source.with_causing_entity(owner);
    }
    Ok(hit.entity.hurt(access.world, &source, amount))
}

struct Handler {
    callback: extern "C" fn(Server<'_>, Impact<'_>, u64),
    token: u64,
    server: Weak<CoreServer>,
}
impl ImpactHandler for Handler {
    fn on_hit(
        &self,
        projectile: &Arc<ExtensionProjectile>,
        world: &Arc<CoreWorld>,
        hit: &ProjectileHit,
    ) {
        let Some(server) = self.server.upgrade() else {
            return;
        };
        (self.callback)(
            Server::from_host(&server),
            Impact::from_host(&ImpactAccess {
                world,
                projectile,
                hit,
            }),
            self.token,
        );
    }
}

pub(crate) fn launch(
    world: &Arc<CoreWorld>,
    id: PlayerId,
    item: &ItemSpec,
    spec: ProjectileSpec,
    target: Option<Position>,
    callback: extern "C" fn(Server<'_>, Impact<'_>, u64),
    token: u64,
) -> Result<i32, String> {
    if !spec.speed.is_finite()
        || spec.speed <= 0.0
        || !spec.uncertainty.is_finite()
        || spec.uncertainty < 0.0
    {
        return Err(
            "projectile speed must be positive and uncertainty nonnegative, both finite".into(),
        );
    }
    let player = world
        .players
        .get_by_uuid(&uuid::Uuid::from_bytes(id.bytes))
        .filter(|p| p.id() == id.entity_id && !p.is_removed())
        .ok_or("shooter is not in this world")?;
    let item = super::items::to_stack(item)?;
    if item.is_empty() {
        return Err("projectile rendering needs a nonempty item".into());
    }
    let pos = player.position();
    // Same spawn offset as the native throwable-item launcher.
    let position = glam::DVec3::new(pos.x, player.get_eye_y() - 0.1, pos.z);
    let target = target.map(|point| glam::DVec3::new(point.x, point.y, point.z));
    if target.is_some_and(|point| !(point - position).is_finite() || point == position) {
        return Err(
            "projectile target must be finite and distinct from the launch position".into(),
        );
    }
    let projectile = ExtensionProjectile::new(
        next_entity_id(),
        position,
        Arc::downgrade(world),
        spec.gravity,
        spec.lifetime,
        Box::new(Handler {
            callback,
            token,
            server: Arc::downgrade(&player.server()),
        }),
    )?;
    let owner: SharedEntity = player;
    projectile.set_owner_entity(Some(&owner));
    projectile.set_item_clamped(item);
    let (yaw, pitch) = if let Some(target) = target {
        projectile.look_at(EntityAnchor::Feet, target);
        projectile.rotation()
    } else {
        owner.rotation()
    };
    projectile.shoot_from_rotation(
        owner.as_ref(),
        pitch,
        yaw,
        0.0,
        spec.speed,
        spec.uncertainty,
    );
    let id = projectile.id();
    world
        .try_add_entity(Arc::new(projectile))
        .map_err(|e| e.to_string())?;
    Ok(id)
}
