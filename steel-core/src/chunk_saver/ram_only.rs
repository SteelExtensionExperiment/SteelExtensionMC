use std::{
    io,
    sync::{Arc, Weak},
};

use rustc_hash::FxHashMap;
use steel_utils::{ChunkPos, locks::AsyncRwLock};
use tokio::sync::oneshot;

use crate::world::{World, template::WorldTemplate};

use super::{ChunkStorage, LoadedChunk, PersistentChunk, PreparedChunkSave};

/// In-memory chunk storage.
///
/// This storage implementation doesn't
/// persist any data to disk. It's designed for test worlds and minigame worlds:
/// - It has chunk generation which can be disabled with `EmptyChunkGen`
/// - It will save all the data so perfectly for minigames
///
/// A world cloned from a [`WorldTemplate`] reads chunks it has not saved through to the shared
/// template, so untouched chunks cost no memory and clones never see each other's edits.
pub struct RamOnlyStorage {
    /// This saves every chunk, and it saves the changes in the world to make it possible to run the server fully in memory
    saved_chunks: AsyncRwLock<FxHashMap<ChunkPos, SimpleRAMChunk>>,
    /// Read-through source for chunks missing from `saved_chunks`.
    base: Option<Arc<WorldTemplate>>,
}

/// Represents a simple in-memory prepared chunk save.
///
/// This structure is used to manage the in-memory representation of a chunk in a system,
/// including its saved data and current processing or usage status.
pub struct SimpleRAMChunk {
    /// A `PreparedChunkSave` instance that holds the saved state of the chunk.
    pub prepared: PreparedChunkSave,
}

impl RamOnlyStorage {
    /// Creates a new RAM-only storage which can be used for minigames, etc.
    ///
    /// This should be used for a RAM storage solution of a map and every world generation should be supported
    #[must_use]
    pub fn empty_world() -> Self {
        Self {
            saved_chunks: AsyncRwLock::new(FxHashMap::default()),
            base: None,
        }
    }

    /// Creates storage that reads unsaved chunks from `template` and keeps its own edits.
    #[must_use]
    pub fn from_template(template: Arc<WorldTemplate>) -> Self {
        Self {
            saved_chunks: AsyncRwLock::new(FxHashMap::default()),
            base: Some(template),
        }
    }

    /// Loads a chunk from storage: the world's own saved chunk first, then the template's.
    pub async fn load_chunk(
        &self,
        pos: ChunkPos,
        min_y: i32,
        height: i32,
        level: Weak<World>,
        thread_pool: &rayon::ThreadPool,
    ) -> io::Result<Option<LoadedChunk>> {
        if let Some(saved) = self.saved_chunks.read().await.get(&pos) {
            return ChunkStorage::try_persistent_to_chunk(
                &saved.prepared.persistent,
                pos,
                saved.prepared.status,
                min_y,
                height,
                level,
            )
            .map(Some);
        }
        let Some(template) = self.base.as_ref().filter(|t| t.chunk(pos).is_some()) else {
            return Ok(None);
        };
        let template = Arc::clone(template);
        // Keep CPU-heavy decoding off the async runtime.
        let (sender, receiver) = oneshot::channel();
        thread_pool.spawn(move || {
            let result = Self::decode_template_chunk(&template, pos, min_y, height, level);
            if sender.send(result).is_err() {
                tracing::trace!(chunk = ?pos, "Discarding decoded template chunk after its load task was canceled");
            }
        });
        receiver
            .await
            .map_err(|_| io::Error::other("template chunk decode task ended without a result"))?
            .map(Some)
    }

    fn decode_template_chunk(
        template: &WorldTemplate,
        pos: ChunkPos,
        min_y: i32,
        height: i32,
        level: Weak<World>,
    ) -> io::Result<LoadedChunk> {
        let Some(chunk) = template.chunk(pos) else {
            return Err(io::Error::other("template chunk disappeared"));
        };
        let invalid = |what: &str, error: &dyn std::fmt::Display| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("template {} chunk {pos:?}: {what}: {error}", template.key),
            )
        };
        let data =
            zstd::decode_all(&chunk.payload[..]).map_err(|e| invalid("zstd decode failed", &e))?;
        let persistent: PersistentChunk<'_> =
            wincode::deserialize(&data).map_err(|e| invalid("chunk decode failed", &e))?;
        ChunkStorage::try_persistent_to_chunk(&persistent, pos, chunk.status, min_y, height, level)
    }

    /// Saves prepared chunk data to storage.
    pub async fn save_chunk_data(&self, prepared: PreparedChunkSave) -> io::Result<bool> {
        // Just track that this chunk has been saved
        // The actual data is in the live World/Chunk, not persisted
        self.saved_chunks
            .write()
            .await
            .insert(prepared.pos, SimpleRAMChunk { prepared });
        Ok(true)
    }

    /// Checks if a chunk exists in storage.
    pub async fn chunk_exists(&self, pos: ChunkPos) -> io::Result<bool> {
        Ok(self.saved_chunks.read().await.contains_key(&pos)
            || self.base.as_ref().is_some_and(|t| t.chunk(pos).is_some()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chunk::{
            Chunk,
            section::{ChunkSection, Sections},
            status::ChunkStatus,
        },
        chunk_saver::format::CHUNK_COMPRESSION_LEVEL,
        level_data::WorldGenerationSettings,
        world::template::TemplateChunk,
    };
    use steel_registry::init_vanilla_registry;
    use steel_utils::Identifier;

    const MIN_Y: i32 = 0;
    const HEIGHT: i32 = 16;

    fn test_pool() -> rayon::ThreadPool {
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("test pool should build")
    }

    fn proto_chunk(pos: ChunkPos) -> Chunk {
        Chunk::new(
            Sections::from_owned(vec![ChunkSection::new_empty()].into_boxed_slice()),
            pos,
            MIN_Y,
            HEIGHT,
            Weak::new(),
        )
    }

    fn prepared(pos: ChunkPos, status: ChunkStatus) -> PreparedChunkSave {
        ChunkStorage::prepare_chunk_save(&proto_chunk(pos), status, &[], true)
            .expect("forced save should prepare")
    }

    fn template_with(chunks: &[(ChunkPos, ChunkStatus)]) -> Arc<WorldTemplate> {
        let chunks = chunks
            .iter()
            .map(|&(pos, status)| {
                let encoded = wincode::serialize(&prepared(pos, status).persistent)
                    .expect("chunk should encode");
                let payload = zstd::encode_all(encoded.as_slice(), CHUNK_COMPRESSION_LEVEL)
                    .expect("chunk should compress");
                (
                    pos,
                    TemplateChunk {
                        status,
                        payload: payload.into(),
                    },
                )
            })
            .collect();
        let generation = WorldGenerationSettings::from_generator_config(
            Identifier::vanilla_static("flat"),
            &toml::Value::Table(toml::map::Map::new()),
            Identifier::vanilla_static("overworld"),
            MIN_Y,
            HEIGHT,
        );
        Arc::new(WorldTemplate::from_chunks(
            Identifier::new_static("test", "template"),
            generation,
            chunks,
        ))
    }

    async fn load(storage: &RamOnlyStorage, pos: ChunkPos) -> Option<LoadedChunk> {
        storage
            .load_chunk(pos, MIN_Y, HEIGHT, Weak::new(), &test_pool())
            .await
            .expect("load should succeed")
    }

    #[tokio::test]
    async fn untouched_template_chunks_are_read_through_without_being_copied() {
        init_vanilla_registry();
        let pos = ChunkPos::new(2, -3);
        let storage = RamOnlyStorage::from_template(template_with(&[(pos, ChunkStatus::Terrain)]));

        let loaded = load(&storage, pos).await.expect("template chunk loads");

        assert_eq!(
            loaded.status,
            ChunkStatus::Terrain,
            "status comes from the template entry"
        );
        assert!(storage.chunk_exists(pos).await.expect("exists"));
        assert!(storage.saved_chunks.read().await.is_empty());
        assert!(load(&storage, ChunkPos::new(9, 9)).await.is_none());
    }

    #[tokio::test]
    async fn the_overlay_wins_and_edits_stay_in_one_clone() {
        init_vanilla_registry();
        let pos = ChunkPos::new(0, 0);
        let template = template_with(&[(pos, ChunkStatus::Terrain)]);
        let edited = RamOnlyStorage::from_template(Arc::clone(&template));
        let other = RamOnlyStorage::from_template(Arc::clone(&template));

        edited
            .save_chunk_data(prepared(pos, ChunkStatus::Biomes))
            .await
            .expect("save");

        assert_eq!(
            load(&edited, pos).await.expect("overlay chunk").status,
            ChunkStatus::Biomes
        );
        assert_eq!(
            load(&other, pos).await.expect("template chunk").status,
            ChunkStatus::Terrain,
            "another clone still sees the template"
        );
        assert_eq!(
            template.chunk(pos).map(|chunk| chunk.status),
            Some(ChunkStatus::Terrain),
            "the template is never written"
        );
    }
}
