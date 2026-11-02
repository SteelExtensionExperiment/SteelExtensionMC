//! Resolve live identities before delegating to native entity mechanics.

use crate::{
    Position,
    effects::Effect,
    events::EntityInfo,
    world_effects::{EntityId, Vitals},
};
use glam::DVec3;
use stabby::vec::Vec as ApiVec;
use steel_core::extension::events::EntityInfo as NativeEntityInfo;
use steel_core::{
    entity::{MobEffectInstance, SharedEntity, damage::DamageSource},
    world::World,
};
use steel_registry::{REGISTRY, RegistryExt, TaggedRegistryExt};
use steel_utils::{Identifier, WorldAabb};

pub(crate) fn find(world: &World, id: EntityId) -> Result<SharedEntity, String> {
    world
        .get_entity_by_id(id.id)
        .filter(|e| e.uuid().as_bytes() == &id.uuid.bytes && !e.is_removed())
        .ok_or_else(|| "entity is no longer present in this world".into())
}
pub(crate) fn near(
    world: &World,
    position: Position,
    radius: f64,
) -> Result<ApiVec<EntityInfo>, String> {
    if !radius.is_finite()
        || !(0.0..=128.0).contains(&radius)
        || ![position.x, position.y, position.z]
            .into_iter()
            .all(f64::is_finite)
    {
        return Err("entity query needs a finite position and radius in 0–128".into());
    }
    let min = DVec3::new(
        position.x - radius,
        position.y - radius,
        position.z - radius,
    );
    let max = DVec3::new(
        position.x + radius,
        position.y + radius,
        position.z + radius,
    );
    Ok(world
        .get_entities_in_aabb_matching(
            &WorldAabb::new(min.x, min.y, min.z, max.x, max.y, max.z),
            |e| !e.is_removed() && e.as_living_entity().is_some(),
        )
        .iter()
        .map(|e| super::events::entity_info(&NativeEntityInfo::of(e.as_ref())))
        .collect())
}
pub(crate) fn vitals(world: &World, id: EntityId) -> Result<Vitals, String> {
    let entity = find(world, id)?;
    let living = entity.as_living_entity().ok_or("entity is not living")?;
    Ok(Vitals {
        health: living.get_health(),
        maximum: living.get_max_health(),
    })
}
pub(crate) fn hurt(
    world: &World,
    id: EntityId,
    cause: &str,
    amount: f32,
    owner: Option<EntityId>,
) -> Result<bool, String> {
    if !amount.is_finite() || amount < 0.0 {
        return Err("damage must be finite and nonnegative".into());
    }
    let key: Identifier = cause.parse().map_err(str::to_owned)?;
    let kind = REGISTRY
        .damage_types
        .by_key(&key)
        .ok_or("unknown damage type")?;
    let mut source = DamageSource::environment(kind);
    if let Some(owner) = owner {
        source = source.with_causing_entity(find(world, owner)?);
    }
    Ok(find(world, id)?.hurt(world, &source, amount))
}
pub(crate) fn affect(world: &World, id: EntityId, effect: &Effect) -> Result<bool, String> {
    let key: Identifier = effect.key.as_str().parse().map_err(str::to_owned)?;
    let kind = REGISTRY
        .mob_effects
        .by_key(&key)
        .ok_or("unknown mob effect")?;
    let instance = MobEffectInstance::with_duration(kind, effect.duration, effect.amplifier)
        .with_ambient(effect.ambient)
        .with_visible(effect.particles)
        .with_show_icon(effect.icon);
    let entity = find(world, id)?;
    Ok(entity
        .as_living_entity()
        .ok_or("entity is not living")?
        .add_mob_effect(instance))
}
pub(crate) fn ignite(world: &World, id: EntityId, ticks: i32) -> Result<(), String> {
    if ticks <= 0 {
        return Err("fire duration must be positive".into());
    }
    find(world, id)?.ignite_for_ticks(ticks);
    Ok(())
}
pub(crate) fn cause_in_tag(cause: &Identifier, tag: &str) -> Result<bool, String> {
    let tag: Identifier = tag.parse().map_err(str::to_owned)?;
    if !REGISTRY.damage_types.tag_keys().any(|key| key == &tag) {
        return Err("unknown damage-type tag".into());
    }
    let kind = REGISTRY
        .damage_types
        .by_key(cause)
        .ok_or("unknown damage cause")?;
    Ok(REGISTRY.damage_types.is_in_tag(kind, &tag))
}
