use super::StoredEntities;
use crate::entities::EntityPolicy;
use glam::DVec3;
use std::sync::Arc;
use steel_core::{
    entity::SharedEntity, entity::custom_data::DataPath, player::Player, world::World,
};
use steel_utils::ChunkPos;

impl StoredEntities {
    /// Returns an entity to chunk persistence, removing its storage record.
    /// Stored entities use the supplied free position; deployments stay in place.
    pub fn release(
        &self,
        player: &Player,
        world: &Arc<World>,
        path: &str,
        position: DVec3,
        policy: EntityPolicy,
    ) -> Result<SharedEntity, String> {
        let data_path = DataPath::parse(path)?;
        if !Arc::ptr_eq(world, &player.get_world()) {
            return Err("release must use the owner's current world".into());
        }
        if !self.deployed.lock().contains_key(path) {
            self.restore(player, world, path, position, policy)?;
        }
        let mut deployments = self.deployed.lock();
        let entry = deployments
            .get(path)
            .ok_or("entity is no longer deployed")?;
        let entity = entry.release(player, &data_path)?;
        deployments.remove(path);
        drop(deployments);
        world.mark_chunk_dirty(ChunkPos::from_entity_pos(entity.position()));
        Ok(entity)
    }
}

#[cfg(test)]
mod tests;
