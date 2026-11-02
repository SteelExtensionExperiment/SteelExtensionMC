pub(crate) mod control;
mod runtime;
use super::{players::find, world_effects};
use crate::players::Uuid;
use crate::{
    PlayerId, Position,
    entities::{EntityPolicy, StoredEntity},
    events::EntityInfo,
    world_effects::EntityId,
};
use glam::DVec3;
use runtime::StoredEntities;
use stabby::vec::Vec as ApiVec;
use std::sync::Arc;
use steel_core::extension::events::EntityInfo as NativeEntityInfo;
use steel_core::{entity::Entity, server::Server, world::World};

pub(crate) fn store(
    world: &Arc<World>,
    id: PlayerId,
    target: EntityId,
    path: &str,
    policy: EntityPolicy,
) -> Result<(), String> {
    let player = world
        .players
        .get_by_uuid(&uuid::Uuid::from_bytes(id.bytes))
        .filter(|p| p.id() == id.entity_id && !p.is_removed())
        .ok_or("player is no longer in the impact world")?;
    StoredEntities::of(&player)?.store(&player, &world_effects::find(world, target)?, path, policy)
}
pub(crate) fn restore(
    server: &Server,
    id: PlayerId,
    path: &str,
    pos: Position,
    policy: EntityPolicy,
) -> Result<EntityInfo, String> {
    let player = find(server, id)?;
    let world = player.get_world();
    let entity = StoredEntities::of(&player)?.restore(
        &player,
        &world,
        path,
        DVec3::new(pos.x, pos.y, pos.z),
        policy,
    )?;
    Ok(super::events::entity_info(&NativeEntityInfo::of(
        entity.as_ref(),
    )))
}
pub(crate) fn release(
    server: &Server,
    id: PlayerId,
    path: &str,
    pos: Position,
    policy: EntityPolicy,
) -> Result<EntityInfo, String> {
    let player = find(server, id)?;
    let entity = StoredEntities::of(&player)?.release(
        &player,
        &player.get_world(),
        path,
        DVec3::new(pos.x, pos.y, pos.z),
        policy,
    )?;
    Ok(super::events::entity_info(&NativeEntityInfo::of(
        entity.as_ref(),
    )))
}
pub(crate) fn synchronize(server: &Server, id: PlayerId) -> Result<(), String> {
    let player = find(server, id)?;
    StoredEntities::of(&player)?.synchronize(&player)
}
pub(crate) fn return_to_storage(server: &Server, id: PlayerId, path: &str) -> Result<bool, String> {
    let player = find(server, id)?;
    StoredEntities::of(&player)?.return_to_storage(&player, path)
}
pub(crate) fn instances(server: &Server, id: PlayerId) -> Result<ApiVec<StoredEntity>, String> {
    let player = find(server, id)?;
    Ok(StoredEntities::of(&player)?
        .deployments()
        .into_iter()
        .map(|(path, entity, instance)| StoredEntity {
            instance: Uuid {
                bytes: *instance.as_bytes(),
            },
            path: path.into(),
            entity: super::events::entity_info(&NativeEntityInfo::of(entity.as_ref())),
        })
        .collect())
}
