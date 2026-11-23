//! Immutable, shareable snapshot of a saved world that runtime clones read from.
use std::{
    fmt,
    path::{Path, PathBuf},
};

use rustc_hash::FxHashMap;
use steel_utils::{ChunkPos, Identifier, types::Difficulty};

use crate::{
    chunk::status::ChunkStatus,
    chunk_saver::RegionReader,
    level_data::{LevelData, SpawnPoint, WorldGenerationSettings},
    worldgen::generator::registry::WorldGeneratorRegistry,
};

/// Name of the region directory inside a world folder.
const REGION_DIRECTORY: &str = "region";

/// One chunk of a template: its persisted status and compressed `PersistentChunk` bytes
/// without entities.
pub struct TemplateChunk {
    /// Status the chunk had when it was saved.
    pub status: ChunkStatus,
    /// Compressed payload as stored in a region file.
    pub payload: Box<[u8]>,
}

/// A saved world read into memory. Never written; shared as `Arc<WorldTemplate>`.
///
/// Game rules, world border, weather, tickets and `data/` are not carried over.
pub struct WorldTemplate {
    /// Registry key chosen by the caller.
    pub key: Identifier,
    /// Source folder, for diagnostics only.
    pub source: PathBuf,
    /// Generator settings the saved world was created with.
    pub generation: WorldGenerationSettings,
    /// Saved world seed, the default for clones.
    pub seed: i64,
    /// Saved world spawn, the default for clones.
    pub spawn: SpawnPoint,
    /// Saved difficulty, the default for clones.
    pub difficulty: Difficulty,
    chunks: FxHashMap<ChunkPos, TemplateChunk>,
}

impl WorldTemplate {
    /// Reads the world folder at `source` (`level.toml` and `region/`) without writing to it.
    ///
    /// # Errors
    /// Returns a message when `level.toml` or a region file cannot be read, the generation
    /// settings are missing, the generator is not registered, or the saved dimension does not
    /// match the generator's.
    pub async fn load(
        key: Identifier,
        source: &Path,
        world_generators: &WorldGeneratorRegistry,
        thread_pool: &rayon::ThreadPool,
    ) -> Result<Self, String> {
        let level_data = LevelData::read_snapshot(source)
            .await
            .map_err(|error| format!("cannot read level data of {}: {error}", source.display()))?;
        let generation = level_data.generation.clone().ok_or_else(|| {
            format!(
                "{} has no generation settings in level.toml, so it cannot be used as a template",
                source.display()
            )
        })?;
        Self::validate_generation(world_generators, &generation)?;

        let region_chunks = RegionReader::read_all(
            &source.join(REGION_DIRECTORY),
            generation.min_y,
            generation.height,
            thread_pool,
        )
        .await
        .map_err(|error| format!("cannot read regions of {}: {error}", source.display()))?;
        let chunks = region_chunks
            .into_iter()
            .map(|(pos, status, payload)| (pos, TemplateChunk { status, payload }))
            .collect();

        Ok(Self {
            key,
            source: source.to_owned(),
            generation,
            seed: level_data.seed,
            spawn: level_data.spawn,
            difficulty: level_data.difficulty,
            chunks,
        })
    }

    /// The saved generator must still be registered with the same dimension type, min Y and height.
    fn validate_generation(
        world_generators: &WorldGeneratorRegistry,
        saved: &WorldGenerationSettings,
    ) -> Result<(), String> {
        let validated = world_generators.validate_config(&saved.generator, &saved.config)?;
        let dimension = validated.dimension_type();
        if dimension.key != saved.dimension_type
            || dimension.min_y != saved.min_y
            || dimension.height != saved.height
        {
            return Err(format!(
                "saved dimension (type {}, min_y {}, height {}) does not match generator {} (type {}, min_y {}, height {})",
                saved.dimension_type,
                saved.min_y,
                saved.height,
                saved.generator,
                dimension.key,
                dimension.min_y,
                dimension.height,
            ));
        }
        Ok(())
    }

    /// The saved chunk at `pos`, if any.
    #[must_use]
    pub fn chunk(&self, pos: ChunkPos) -> Option<&TemplateChunk> {
        self.chunks.get(&pos)
    }

    /// Number of saved chunks.
    #[must_use]
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Bytes held by chunk payloads.
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.chunks.values().map(|chunk| chunk.payload.len()).sum()
    }
}

impl fmt::Debug for WorldTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorldTemplate")
            .field("key", &self.key)
            .field("source", &self.source)
            .field("chunks", &self.chunks.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env, fs, process,
        time::{SystemTime, UNIX_EPOCH},
    };
    use steel_registry::{init_vanilla_registry, vanilla_dimension_types::OVERWORLD};

    fn temp_world_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let path =
            env::temp_dir().join(format!("steel-template-{name}-{}-{unique}", process::id()));
        fs::create_dir_all(&path).expect("temp dir should be created");
        path
    }

    fn flat_settings(min_y: i32, height: i32) -> WorldGenerationSettings {
        let mut config = toml::map::Map::new();
        config.insert(
            "dimension_type".to_owned(),
            toml::Value::String(OVERWORLD.key.to_string()),
        );
        WorldGenerationSettings::from_generator_config(
            Identifier::vanilla_static("flat"),
            &toml::Value::Table(config),
            OVERWORLD.key.clone(),
            min_y,
            height,
        )
    }

    fn write_level(dir: &Path, generation: Option<WorldGenerationSettings>) {
        let mut data = LevelData::new_with_seed(77);
        data.generation = generation;
        data.save_game_rules();
        fs::write(
            dir.join("level.toml"),
            toml::to_string_pretty(&data).expect("level data should serialize"),
        )
        .expect("level.toml should be written");
    }

    async fn load(dir: &Path) -> Result<WorldTemplate, String> {
        init_vanilla_registry();
        let generators =
            WorldGeneratorRegistry::new_with_builtins().expect("builtin generators register");
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("pool");
        WorldTemplate::load(
            Identifier::new_static("test", "template"),
            dir,
            &generators,
            &pool,
        )
        .await
    }

    #[tokio::test]
    async fn folder_without_region_files_is_a_valid_empty_template() {
        init_vanilla_registry();
        let dir = temp_world_dir("empty");
        write_level(&dir, Some(flat_settings(OVERWORLD.min_y, OVERWORLD.height)));

        let template = load(&dir).await.expect("empty template should load");
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(template.chunk_count(), 0);
        assert_eq!(template.seed, 77);
        assert!(!dir.join("region").exists());
    }

    #[tokio::test]
    async fn missing_generation_settings_fail() {
        init_vanilla_registry();
        let dir = temp_world_dir("no-generation");
        write_level(&dir, None);

        let error = load(&dir)
            .await
            .expect_err("template needs generation settings");
        let _ = fs::remove_dir_all(&dir);

        assert!(error.contains("generation settings"), "{error}");
    }

    #[tokio::test]
    async fn unregistered_generator_fails() {
        init_vanilla_registry();
        let dir = temp_world_dir("unknown-generator");
        let mut settings = flat_settings(OVERWORLD.min_y, OVERWORLD.height);
        settings.generator = Identifier::new_static("nobody", "generator");
        write_level(&dir, Some(settings));

        let error = load(&dir).await.expect_err("generator must be registered");
        let _ = fs::remove_dir_all(&dir);

        assert!(error.contains("unknown world generator"), "{error}");
    }

    #[tokio::test]
    async fn dimension_mismatch_fails() {
        init_vanilla_registry();
        let dir = temp_world_dir("dimension-mismatch");
        write_level(
            &dir,
            Some(flat_settings(OVERWORLD.min_y, OVERWORLD.height + 16)),
        );

        let error = load(&dir)
            .await
            .expect_err("dimension must match the generator");
        let _ = fs::remove_dir_all(&dir);

        assert!(error.contains("does not match generator"), "{error}");
    }
}
