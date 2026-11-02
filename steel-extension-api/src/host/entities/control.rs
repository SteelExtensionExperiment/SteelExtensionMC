//! Adapts guest decisions to the core's generic, cancellable AI controller.

use super::super::{events::entity_info, world_effects::find};
use crate::{Position, entities::EntityControl, events::EntityInfo, world_effects::EntityId};
use glam::DVec3;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use steel_core::extension::events::EntityInfo as NativeEntityInfo;
use steel_core::{
    entity::{MobAiController, SharedEntity},
    world::World,
};

pub(crate) struct ControlAccess<'a> {
    pub world: &'a Arc<World>,
    pub entity: &'a SharedEntity,
}

pub(crate) fn info(world: &World, id: EntityId) -> Result<EntityInfo, String> {
    Ok(entity_info(&NativeEntityInfo::of(
        find(world, id)?.as_ref(),
    )))
}
pub(crate) fn has_ai(world: &World, id: EntityId) -> Result<bool, String> {
    Ok(find(world, id)?.as_mob().is_some())
}
pub(crate) fn current(access: &ControlAccess<'_>) -> EntityInfo {
    entity_info(&NativeEntityInfo::of(access.entity.as_ref()))
}
pub(crate) fn can_move(access: &ControlAccess<'_>) -> bool {
    access
        .entity
        .as_mob()
        .is_some_and(|mob| mob.as_pathfinder_mob().is_some() || mob.flight_control().is_some())
}
pub(crate) fn move_to(
    access: &ControlAccess<'_>,
    target: EntityId,
    speed: f64,
    repath: bool,
) -> Result<(), String> {
    if !speed.is_finite() || speed <= 0.0 {
        return Err("movement speed must be finite and positive".into());
    }
    let mob = access.entity.as_mob().ok_or("entity has no mob AI")?;
    let target = find(access.world, target)?;
    let position = target.position();
    if let Some(pathfinder) = mob.as_pathfinder_mob() {
        mob.mob_base().controls().lock().look_control.set_look_at(
            DVec3::new(position.x, target.get_eye_y(), position.z),
            30.0,
            30.0,
        );
        if repath {
            pathfinder.move_to_pos(position, speed);
        }
    } else if let Some(flight) = mob.flight_control() {
        let center = target.bounding_box().center();
        mob.mob_base()
            .controls()
            .lock()
            .look_control
            .set_look_at(center, 30.0, 30.0);
        flight.fly_towards(center);
    } else {
        return Err("entity has no supported movement capability".into());
    }
    Ok(())
}
pub(crate) fn look_at(
    access: &ControlAccess<'_>,
    position: Position,
    yaw_limit: f32,
    pitch_limit: f32,
) -> Result<(), String> {
    let position = DVec3::new(position.x, position.y, position.z);
    if !position.is_finite()
        || !yaw_limit.is_finite()
        || !pitch_limit.is_finite()
        || yaw_limit < 0.0
        || pitch_limit < 0.0
    {
        return Err(
            "look position and angular limits must be finite; limits must be nonnegative".into(),
        );
    }
    let mob = access.entity.as_mob().ok_or("entity has no mob AI")?;
    mob.mob_base()
        .controls()
        .lock()
        .look_control
        .set_look_at(position, yaw_limit, pitch_limit);
    Ok(())
}
pub(crate) fn can_melee_attack(
    access: &ControlAccess<'_>,
    target: EntityId,
) -> Result<bool, String> {
    let mob = access.entity.as_mob().ok_or("entity has no mob AI")?;
    let target = find(access.world, target)?;
    Ok(target.as_living_entity().is_some_and(|living| {
        mob.is_within_melee_attack_range(living) && mob.has_line_of_sight_cached(target.as_ref())
    }))
}

#[derive(Debug)]
struct Controller {
    callback: extern "C" fn(EntityControl<'_>, u64) -> bool,
    token: u64,
    finished: AtomicBool,
}
impl MobAiController for Controller {
    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
    fn tick(&self, entity: &SharedEntity) {
        let Some(world) = entity.level() else {
            self.finished.store(true, Ordering::Release);
            return;
        };
        if !(self.callback)(
            EntityControl::from_host(&ControlAccess {
                world: &world,
                entity,
            }),
            self.token,
        ) {
            self.finished.store(true, Ordering::Release);
            self.stop(entity);
            return;
        }
        if let Some(mob) = entity.as_mob() {
            mob.tick_path_navigation();
        }
    }
    fn stop(&self, entity: &SharedEntity) {
        if let Some(mob) = entity.as_mob() {
            mob.mob_base().navigation().lock().stop();
        }
    }
}
pub(crate) fn install(
    world: &World,
    id: EntityId,
    callback: extern "C" fn(EntityControl<'_>, u64) -> bool,
    token: u64,
) -> Result<(), String> {
    let entity = find(world, id)?;
    let mob = entity.as_mob().ok_or("entity has no mob AI")?;
    mob.mob_base().ai_controller().install(Arc::new(Controller {
        callback,
        token,
        finished: AtomicBool::new(false),
    }))
}
pub(crate) fn is_controlled(world: &World, id: EntityId) -> Result<bool, String> {
    let entity = find(world, id)?;
    Ok(entity
        .as_mob()
        .is_some_and(|mob| mob.mob_base().ai_controller().is_active()))
}

pub(crate) fn hurt_target(
    access: &ControlAccess<'_>,
    target: EntityId,
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
    let source = DamageSource::direct(kind, Arc::clone(access.entity));
    Ok(find(access.world, target)?.hurt(access.world, &source, amount))
}
