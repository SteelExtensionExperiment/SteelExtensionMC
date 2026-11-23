//! Text holograms: a per-world record that outlives the chunk-bound text display showing it.
//!
//! The entity is not persistent. It is spawned when its chunk loads and dropped with the chunk,
//! so a hologram has at most one live entity and none while its chunk is unloaded.

use std::sync::{
    Arc, Weak,
    atomic::{AtomicU64, Ordering},
};

use glam::DVec3;
use rustc_hash::FxHashMap;
use steel_utils::{BlockPos, ChunkPos, locks::SyncMutex};
use text_components::TextComponent;
use thiserror::Error;

use super::{LevelReader as _, World};
use crate::entity::{
    Entity as _, RemovalReason, SharedEntity,
    entities::{BillboardConstraints, Display as _, DisplayView as _, TextDisplayEntity},
    next_entity_id,
};

/// Source of [`HologramId`]s. Shared by all worlds so an id is never reused in a process.
static NEXT_HOLOGRAM_ID: AtomicU64 = AtomicU64::new(0);

/// Opaque handle of a hologram, unique across worlds for the lifetime of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HologramId(u64);

impl HologramId {
    /// Rebuilds an id from [`Self::as_raw`]. An id that was never issued names no hologram.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw value of this id.
    #[must_use]
    pub const fn as_raw(self) -> u64 {
        self.0
    }
}

/// Why a hologram operation failed.
#[derive(Debug, Error)]
pub enum HologramError {
    /// The position has a NaN or infinite coordinate.
    #[error("hologram position {0} is not finite")]
    NonFinitePosition(DVec3),
    /// The position lies outside the dimension's build height.
    #[error("hologram y {y} is outside the build height of this world")]
    OutsideBuildHeight {
        /// The rejected y coordinate.
        y: f64,
    },
    /// No hologram with this id exists in this world.
    #[error("unknown or removed hologram")]
    Unknown,
}

struct Hologram {
    position: DVec3,
    text: TextComponent,
    billboard: BillboardConstraints,
    /// `None` while the chunk is unloaded.
    entity: Option<Arc<TextDisplayEntity>>,
}

impl Hologram {
    fn has_live_entity(&self) -> bool {
        self.entity
            .as_ref()
            .is_some_and(|entity| !entity.is_removed())
    }
}

#[derive(Default)]
struct HologramState {
    holograms: FxHashMap<HologramId, Hologram>,
    by_chunk: FxHashMap<ChunkPos, Vec<HologramId>>,
}

/// The holograms of one world, dropped together with it.
///
/// Lock order: this lock is never held while calling into the entity manager, entity
/// registration or entity removal, because those can run extension code that re-enters
/// the world. It may be held while locking an entity's own data mutex.
#[derive(Default)]
pub(super) struct HologramRegistry(SyncMutex<HologramState>);

impl HologramRegistry {
    fn ids_in_chunk(&self, chunk: ChunkPos) -> Vec<HologramId> {
        self.0
            .lock()
            .by_chunk
            .get(&chunk)
            .cloned()
            .unwrap_or_default()
    }

    /// Builds the entity for a hologram that has no live entity.
    fn build_entity(&self, id: HologramId, world: Weak<World>) -> Option<Arc<TextDisplayEntity>> {
        let state = self.0.lock();
        let hologram = state.holograms.get(&id)?;
        if hologram.has_live_entity() {
            return None;
        }
        let entity = Arc::new(TextDisplayEntity::new_hologram(
            next_entity_id(),
            hologram.position,
            world,
        ));
        entity.with_view(|mut view| {
            view.set_text(hologram.text.clone());
            view.set_billboard_constraints(hologram.billboard);
        });
        Some(entity)
    }

    /// Records a freshly registered entity, or hands it back if the hologram was removed
    /// or another path already attached one.
    fn attach_entity(
        &self,
        id: HologramId,
        entity: Arc<TextDisplayEntity>,
    ) -> Result<(), Arc<TextDisplayEntity>> {
        let mut state = self.0.lock();
        let Some(hologram) = state.holograms.get_mut(&id) else {
            return Err(entity);
        };
        if hologram.has_live_entity() {
            return Err(entity);
        }
        // The text may have changed while the entity was being registered.
        entity.with_view(|mut view| view.set_text(hologram.text.clone()));
        hologram.entity = Some(entity);
        Ok(())
    }

    fn forget_unloaded_entities(&self, chunk: ChunkPos) {
        let mut state = self.0.lock();
        let HologramState {
            holograms,
            by_chunk,
        } = &mut *state;
        let Some(ids) = by_chunk.get(&chunk) else {
            return;
        };
        for id in ids {
            let Some(hologram) = holograms.get_mut(id) else {
                continue;
            };
            if hologram
                .entity
                .as_ref()
                .is_some_and(|entity| entity.is_removed())
            {
                hologram.entity = None;
            }
        }
    }
}

impl World {
    /// Creates a text hologram and returns its handle.
    ///
    /// The record is always stored. The entity is spawned now only if the chunk is already
    /// loaded; otherwise it appears when the chunk loads. No chunk is ever force-loaded.
    ///
    /// # Errors
    ///
    /// Returns an error if `position` is not finite or lies outside the build height.
    pub fn create_hologram(
        self: &Arc<Self>,
        position: DVec3,
        text: TextComponent,
        billboard: BillboardConstraints,
    ) -> Result<HologramId, HologramError> {
        if !position.is_finite() {
            return Err(HologramError::NonFinitePosition(position));
        }
        let block = BlockPos::containing(position.x, position.y, position.z);
        if self.is_outside_build_height(block.y()) {
            return Err(HologramError::OutsideBuildHeight { y: position.y });
        }

        let id = HologramId(NEXT_HOLOGRAM_ID.fetch_add(1, Ordering::Relaxed));
        let chunk = ChunkPos::from_entity_pos(position);
        {
            let mut state = self.holograms.0.lock();
            state.by_chunk.entry(chunk).or_default().push(id);
            state.holograms.insert(
                id,
                Hologram {
                    position,
                    text,
                    billboard,
                    entity: None,
                },
            );
        }
        // The record is stored before the chunk is checked, and the load hook registers the
        // chunk before it reads the records, so one of the two always spawns the entity.
        if self.entity_manager.is_chunk_loaded(chunk) {
            self.spawn_hologram_entity(id, chunk);
        }
        Ok(id)
    }

    /// Replaces the text of a hologram. A live entity sends one entity-data update.
    ///
    /// # Errors
    ///
    /// Returns [`HologramError::Unknown`] if the hologram does not exist in this world.
    pub fn set_hologram_text(
        &self,
        id: HologramId,
        text: TextComponent,
    ) -> Result<(), HologramError> {
        let mut state = self.holograms.0.lock();
        let hologram = state.holograms.get_mut(&id).ok_or(HologramError::Unknown)?;
        hologram.text = text;
        if let Some(entity) = &hologram.entity {
            entity.with_view(|mut view| view.set_text(hologram.text.clone()));
        }
        Ok(())
    }

    /// Removes a hologram and its entity. Returns whether it existed in this world.
    pub fn remove_hologram(&self, id: HologramId) -> bool {
        let hologram = {
            let mut state = self.holograms.0.lock();
            let Some(hologram) = state.holograms.remove(&id) else {
                return false;
            };
            let chunk = ChunkPos::from_entity_pos(hologram.position);
            if let Some(ids) = state.by_chunk.get_mut(&chunk) {
                ids.retain(|other| *other != id);
                if ids.is_empty() {
                    state.by_chunk.remove(&chunk);
                }
            }
            hologram
        };
        if let Some(entity) = hologram.entity {
            entity.set_removed(RemovalReason::Discarded);
        }
        true
    }

    /// Spawns the entities of the holograms in a chunk whose entities just became loadable.
    pub(super) fn spawn_chunk_holograms(self: &Arc<Self>, chunk: ChunkPos) {
        for id in self.holograms.ids_in_chunk(chunk) {
            self.spawn_hologram_entity(id, chunk);
        }
    }

    /// Clears the entity of every hologram in a chunk whose entities were just dropped.
    pub(super) fn forget_chunk_hologram_entities(&self, chunk: ChunkPos) {
        self.holograms.forget_unloaded_entities(chunk);
    }

    fn spawn_hologram_entity(self: &Arc<Self>, id: HologramId, chunk: ChunkPos) {
        let Some(entity) = self.holograms.build_entity(id, Arc::downgrade(self)) else {
            return;
        };
        let shared = Arc::clone(&entity) as SharedEntity;
        // `try_add_entity` needs a full chunk. The load hook runs earlier, at Empty status,
        // where entities restored from disk use `register_loaded_entity` as well.
        let added = if self.has_full_chunk(chunk) {
            self.try_add_entity(shared)
        } else {
            self.register_loaded_entity(shared)
        };
        if let Err(error) = added {
            // The record stays; the next chunk load tries again.
            tracing::warn!(?chunk, "Could not spawn hologram entity: {error:?}");
            return;
        }
        if let Err(extra) = self.holograms.attach_entity(id, entity) {
            extra.set_removed(RemovalReason::Discarded);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::init_behaviors;
    use crate::test_support::{TestWorld, fresh_test_world, insert_ready_full_chunk};
    use steel_registry::init_vanilla_registry;
    use text_components::TextComponent;

    const POSITION: DVec3 = DVec3::new(8.5, 64.0, 8.5);

    fn fixture(key: &'static str) -> TestWorld {
        init_vanilla_registry();
        init_behaviors();
        fresh_test_world(key)
    }

    fn chunk() -> ChunkPos {
        ChunkPos::from_entity_pos(POSITION)
    }

    fn text_of(entity: &TextDisplayEntity) -> TextComponent {
        let mut text = None;
        entity.with_view(|view| text = Some(*view.text()));
        text.expect("with_view runs its closure")
    }

    fn hologram_entities(world: &World) -> Vec<SharedEntity> {
        world.entity_manager.live_entities_in_chunk(chunk())
    }

    fn unload(world: &Arc<World>) {
        world.on_entity_chunk_unload_start(chunk());
        world.chunk_map.chunks.remove_sync(&chunk());
        world.unregister_full_chunk_ticks(chunk());
        world.on_entity_chunk_unload_finalized(chunk());
    }

    fn create(world: &Arc<World>, text: &str) -> HologramId {
        world
            .create_hologram(
                POSITION,
                text.to_owned().into(),
                BillboardConstraints::Center,
            )
            .expect("hologram in range")
    }

    #[test]
    fn exactly_one_entity_survives_unload_and_load_cycles() {
        let fixture = fixture("hologram_cycles");
        let world = &fixture.world;
        insert_ready_full_chunk(world, chunk());
        let id = create(world, "stats");
        assert_eq!(hologram_entities(world).len(), 1);

        for _ in 0..3 {
            unload(world);
            assert!(hologram_entities(world).is_empty());
            assert!(world.holograms.0.lock().holograms[&id].entity.is_none());
            insert_ready_full_chunk(world, chunk());
            assert_eq!(hologram_entities(world).len(), 1);
        }
    }

    #[test]
    fn text_set_while_unloaded_shows_after_load() {
        let fixture = fixture("hologram_text_unloaded");
        let world = &fixture.world;
        insert_ready_full_chunk(world, chunk());
        let id = create(world, "before");
        unload(world);

        world
            .set_hologram_text(id, "after".into())
            .expect("record still exists");
        insert_ready_full_chunk(world, chunk());

        let state = world.holograms.0.lock();
        let entity = state.holograms[&id].entity.as_ref().expect("respawned");
        assert_eq!(text_of(entity), TextComponent::from("after"));
    }

    #[test]
    fn text_set_while_loaded_updates_the_live_entity() {
        let fixture = fixture("hologram_text_loaded");
        let world = &fixture.world;
        insert_ready_full_chunk(world, chunk());
        let id = create(world, "before");

        world
            .set_hologram_text(id, "after".into())
            .expect("record exists");

        let state = world.holograms.0.lock();
        let entity = state.holograms[&id].entity.as_ref().expect("live");
        assert_eq!(text_of(entity), TextComponent::from("after"));
    }

    #[test]
    fn create_without_chunk_stores_record_and_spawns_on_load() {
        let fixture = fixture("hologram_chunkless");
        let world = &fixture.world;
        let id = create(world, "late");
        assert!(world.holograms.0.lock().holograms[&id].entity.is_none());

        insert_ready_full_chunk(world, chunk());

        assert_eq!(hologram_entities(world).len(), 1);
        assert!(world.holograms.0.lock().holograms[&id].entity.is_some());
    }

    #[test]
    fn removed_hologram_cannot_be_updated_and_loses_its_entity() {
        let fixture = fixture("hologram_remove");
        let world = &fixture.world;
        insert_ready_full_chunk(world, chunk());
        let id = create(world, "gone");

        assert!(world.remove_hologram(id));

        assert!(hologram_entities(world).is_empty());
        assert!(!world.remove_hologram(id));
        assert!(matches!(
            world.set_hologram_text(id, "again".into()),
            Err(HologramError::Unknown)
        ));
    }

    #[test]
    fn positions_outside_the_world_are_rejected() {
        let fixture = fixture("hologram_range");
        let world = &fixture.world;
        let above = DVec3::new(0.0, f64::from(world.max_y_exclusive()), 0.0);
        let below = DVec3::new(0.0, f64::from(world.min_y()) - 1.0, 0.0);
        let nan = DVec3::new(f64::NAN, 64.0, 0.0);

        for position in [above, below] {
            assert!(matches!(
                world.create_hologram(position, "x".into(), BillboardConstraints::Fixed),
                Err(HologramError::OutsideBuildHeight { .. })
            ));
        }
        assert!(matches!(
            world.create_hologram(nan, "x".into(), BillboardConstraints::Fixed),
            Err(HologramError::NonFinitePosition(_))
        ));
    }
}
