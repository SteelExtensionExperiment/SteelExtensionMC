//! Host side of mob hooks and spawning.

use std::sync::Arc;

use glam::DVec3;
use steel_core::entity::entities::mobs::extension::{MobHookRegistry, MobHooks};
use steel_core::entity::{ENTITIES, Entity as CoreEntity, SharedEntity, next_entity_id};
use steel_core::world::World as CoreWorld;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::{REGISTRY, RegistryExt as _};
use steel_utils::{Identifier, WorldAabb};

use super::HookRegistration;
use crate::block_behavior::types::Entity;
use crate::entity_behavior::{DynEntityBehavior, EntityBehaviorDyn, api::EntityBehaviors};
use crate::players::Uuid;
use crate::{ApiResult, Position, World};

pub(crate) struct EntityRegistration<'a> {
    pub registry: &'a mut MobHookRegistry,
    pub config: &'a str,
}

pub(crate) fn subscribe(
    hooks: &mut HookRegistration<'_>,
    callback: extern "C" fn(EntityBehaviors<'_>) -> ApiResult<()>,
) -> Result<(), String> {
    hooks.extension.begin_registration();
    let extension = Arc::clone(hooks.extension);
    let result = MobHookRegistry::on_registration(move |registry| {
        extension.run_registration("entity", || {
            let mut access = EntityRegistration {
                registry,
                config: &extension.config,
            };
            callback(EntityBehaviors::from_host(&mut access))
        });
    });
    if result.is_err() {
        hooks.extension.cancel_registration();
    }
    result
}

fn entity_type(key: &str) -> Result<EntityTypeRef, String> {
    let key = key.parse::<Identifier>().map_err(ToString::to_string)?;
    REGISTRY
        .entity_types
        .by_key(&key)
        .ok_or_else(|| format!("unknown entity type {key}"))
}

pub(crate) fn set_entity_behavior(
    registration: &mut EntityRegistration<'_>,
    key: &str,
    behavior: DynEntityBehavior,
) -> Result<(), String> {
    let key = key.parse::<Identifier>().map_err(ToString::to_string)?;
    registration
        .registry
        .set_hooks(&key, Arc::new(Adapter(behavior)))
}

pub(crate) fn spawn_entity(
    world: &Arc<CoreWorld>,
    key: &str,
    position: Position,
) -> Result<Uuid, String> {
    let entity_type = entity_type(key)?;
    if !entity_type.summonable {
        return Err(format!("{} cannot be spawned", entity_type.key));
    }
    let position = DVec3::new(position.x, position.y, position.z);
    let entity: SharedEntity = ENTITIES
        .create(
            entity_type,
            next_entity_id(),
            position,
            Arc::downgrade(world),
        )
        .ok_or_else(|| format!("Steel cannot create {} yet", entity_type.key))?;
    let uuid = entity.uuid();
    world
        .try_add_entity(entity)
        .map_err(|error| format!("could not add {}: {error:?}", entity_type.key))?;
    Ok(Uuid {
        bytes: *uuid.as_bytes(),
    })
}

pub(crate) fn count_entities(
    world: &Arc<CoreWorld>,
    key: &str,
    center: Position,
    radius: f64,
) -> Result<u32, String> {
    if !radius.is_finite() || radius < 0.0 {
        return Err(format!(
            "radius {radius} must be a finite, non-negative number"
        ));
    }
    let entity_type = entity_type(key)?;
    let center = DVec3::new(center.x, center.y, center.z);
    let area = WorldAabb::new(
        center.x - radius,
        center.y - radius,
        center.z - radius,
        center.x + radius,
        center.y + radius,
        center.z + radius,
    );
    let found = world.get_entities_in_aabb_matching(&area, |entity| {
        std::ptr::eq(entity.entity_type(), entity_type)
            && !entity.is_removed()
            && entity.position().distance_squared(center) <= radius * radius
    });
    Ok(u32::try_from(found.len()).unwrap_or(u32::MAX))
}

/// Calls an extension's hooks from the host's mob.
struct Adapter(DynEntityBehavior);

impl MobHooks for Adapter {
    fn tick(&self, world: &Arc<CoreWorld>, mob: &dyn CoreEntity) {
        self.0.tick(World::from_host(world), Entity::from_host(mob));
    }

    fn attacked(&self, world: &Arc<CoreWorld>, mob: &dyn CoreEntity, target: &dyn CoreEntity) {
        self.0.attacked(
            World::from_host(world),
            Entity::from_host(mob),
            Entity::from_host(target),
        );
    }

    fn died(&self, world: &Arc<CoreWorld>, mob: &dyn CoreEntity) {
        self.0.died(World::from_host(world), Entity::from_host(mob));
    }
}
