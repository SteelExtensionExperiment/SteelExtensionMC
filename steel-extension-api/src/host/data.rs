//! One custom-data store for players and all other loaded entities.

use super::{players, world_effects};
use crate::{PlayerId, entities::EntityId};
use simdnbt::owned::NbtTag;
use std::sync::{Arc, Weak};
use steel_core::{
    entity::{
        Entity, EntityGeneration, SharedEntity,
        custom_data::{CustomData, DataPath},
    },
    server::Server,
    world::World,
};
use steel_utils::ChunkPos;

/// Weak targets cannot keep entities loaded or write to replaced player incarnations.
pub(crate) enum Target {
    Player {
        server: Weak<Server>,
        player: PlayerId,
        generation: EntityGeneration,
    },
    Entity {
        world: Weak<World>,
        entity: EntityId,
        generation: EntityGeneration,
    },
}

impl Target {
    pub(crate) fn player(server: &Arc<Server>, player: PlayerId) -> Result<Self, String> {
        let generation = players::find(server, player)?.generation();
        Ok(Self::Player {
            server: Arc::downgrade(server),
            player,
            generation,
        })
    }

    pub(crate) fn entity(world: &Arc<World>, entity: EntityId) -> Result<Self, String> {
        let generation = world_effects::find(world, entity)?.generation();
        Ok(Self::Entity {
            world: Arc::downgrade(world),
            entity,
            generation,
        })
    }

    fn resolve(&self) -> Result<SharedEntity, String> {
        let (entity, generation) = match self {
            Self::Player {
                server,
                player,
                generation,
            } => {
                let server = server.upgrade().ok_or("data target's server has stopped")?;
                (players::find(&server, *player)? as SharedEntity, generation)
            }
            Self::Entity {
                world,
                entity,
                generation,
            } => {
                let world = world
                    .upgrade()
                    .ok_or("data target's world is unavailable")?;
                (world_effects::find(&world, *entity)?, generation)
            }
        };
        if entity.generation() != *generation {
            return Err("data target has been replaced; open a new store".into());
        }
        Ok(entity)
    }

    pub(crate) fn get(&self, path: &str) -> Result<Option<NbtTag>, String> {
        let path = DataPath::parse(path)?;
        let entity = self.resolve()?;
        let value = entity.data().get(&path).cloned();
        Ok(value)
    }

    pub(crate) fn update<T>(
        &self,
        operation: impl FnOnce(&mut CustomData) -> Result<T, String>,
    ) -> Result<T, String> {
        let entity = self.resolve()?;
        let (result, changed) = {
            let mut data = entity.data();
            let before = data.revision();
            let result = operation(&mut data);
            (result, before != data.revision())
        };
        if changed
            && entity.as_player().is_none()
            && let Some(world) = entity.level()
        {
            world.mark_chunk_dirty(ChunkPos::from_entity_pos(entity.position()));
        }
        result
    }
}

#[cfg(test)]
mod tests;
