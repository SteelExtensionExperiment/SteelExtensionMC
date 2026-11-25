//! Strict read-only reader for a world's `region/` directory.
//!
//! Unlike [`RegionManager`](super::RegionManager) this never creates, renames, truncates or deletes
//! anything: a wrong magic or version, a damaged chunk table or an undecodable chunk fails the
//! whole read. Entities are dropped from the returned payloads; everything else is kept.

use std::{io, path::Path};

use rayon::iter::{IntoParallelIterator, ParallelIterator};
use steel_utils::ChunkPos;
use tokio::{fs, sync::oneshot};

use super::{
    ChunkStorage, PersistentChunk,
    format::{
        CHUNK_COMPRESSION_LEVEL, CHUNK_TABLE_SIZE, FILE_HEADER_SIZE, FORMAT_VERSION, REGION_MAGIC,
        REGION_SIZE, RegionHeader, RegionPos, SECTOR_SIZE, TOTAL_HEADER_SIZE,
    },
};
use crate::chunk::status::ChunkStatus;

/// A chunk read from a region file: position, persisted status and the compressed
/// `PersistentChunk` payload without entities.
pub type RegionChunk = (ChunkPos, ChunkStatus, Box<[u8]>);

/// Reads every region file of a world without modifying the directory.
pub struct RegionReader;

impl RegionReader {
    /// Reads all `r.<x>.<z>.srg` files in `region_dir`, validating every chunk against the world
    /// range (`min_y`, `height`) on `thread_pool`.
    ///
    /// Files with other names are ignored. A missing directory or a directory without region
    /// files yields no chunks and is not created.
    ///
    /// # Errors
    /// Returns an error naming the region (and chunk) when a header or chunk table is invalid, a
    /// file has a different magic or [`FORMAT_VERSION`], or a chunk cannot be decoded.
    pub async fn read_all(
        region_dir: &Path,
        min_y: i32,
        height: i32,
        thread_pool: &rayon::ThreadPool,
    ) -> io::Result<Vec<RegionChunk>> {
        let mut entries = match fs::read_dir(region_dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };

        let mut regions = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let file_name = entry.file_name();
            let Some(region_pos) = file_name.to_str().and_then(RegionPos::from_filename) else {
                tracing::debug!(file = ?file_name, "ignoring non-region file in region directory");
                continue;
            };
            regions.push(region_pos);
        }
        regions.sort_unstable_by_key(|pos| (pos.x, pos.z));

        let mut chunks = Vec::new();
        for region_pos in regions {
            let bytes = fs::read(region_dir.join(region_pos.filename())).await?;
            // Keep CPU-heavy decoding off the async runtime.
            let (sender, receiver) = oneshot::channel();
            thread_pool.spawn(move || {
                let result = Self::decode_region(region_pos, &bytes, min_y, height);
                if sender.send(result).is_err() {
                    tracing::trace!(region = ?region_pos, "discarding region read after cancellation");
                }
            });
            let region_chunks = receiver
                .await
                .map_err(|_| io::Error::other("region decode task ended without a result"))??;
            chunks.extend(region_chunks);
        }
        Ok(chunks)
    }

    fn decode_region(
        region_pos: RegionPos,
        bytes: &[u8],
        min_y: i32,
        height: i32,
    ) -> io::Result<Vec<RegionChunk>> {
        let file_name = region_pos.filename();
        let region_error = |message: String| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("region {file_name}: {message}"),
            )
        };

        if bytes.len() < TOTAL_HEADER_SIZE {
            return Err(region_error(format!(
                "file has {} bytes, shorter than the {TOTAL_HEADER_SIZE} byte header",
                bytes.len()
            )));
        }
        if bytes[0..REGION_MAGIC.len()] != REGION_MAGIC {
            return Err(region_error("invalid region file magic".to_owned()));
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != FORMAT_VERSION {
            return Err(region_error(format!(
                "format version {version}, expected {FORMAT_VERSION}"
            )));
        }

        let header = RegionHeader::from_bytes(&bytes[FILE_HEADER_SIZE..TOTAL_HEADER_SIZE])
            .map_err(|index| {
                region_error(format!(
                    "chunk table entry {index} has an invalid status byte"
                ))
            })?;
        debug_assert_eq!(TOTAL_HEADER_SIZE - FILE_HEADER_SIZE, CHUNK_TABLE_SIZE);
        let file_sectors = bytes.len().div_ceil(SECTOR_SIZE) as u32;
        header
            .validate_entries(file_sectors)
            .map_err(|error| region_error(error.to_string()))?;

        let existing: Vec<usize> = (0..header.entries.len())
            .filter(|&index| header.entries[index].exists())
            .collect();
        existing
            .into_par_iter()
            .map(|index| {
                let (local_x, local_z) = RegionHeader::index_to_local(index);
                let chunk_x = region_pos.x * REGION_SIZE as i32 + local_x as i32;
                let chunk_z = region_pos.z * REGION_SIZE as i32 + local_z as i32;
                let chunk_error = |message: String| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("region {file_name} chunk ({chunk_x}, {chunk_z}): {message}"),
                    )
                };

                let entry = header.entries[index];
                let start = entry.sector_offset as usize * SECTOR_SIZE;
                let compressed = bytes
                    .get(start..start + entry.size_bytes as usize)
                    .ok_or_else(|| {
                        chunk_error("payload extends past the end of the file".to_owned())
                    })?;
                let payload = Self::strip_entities(compressed, min_y, height)
                    .map_err(|error| chunk_error(error.to_string()))?;
                Ok((ChunkPos::new(chunk_x, chunk_z), entry.status, payload))
            })
            .collect()
    }

    /// Validates the payload and returns it without entities. Payloads that already have none
    /// are returned byte for byte.
    fn strip_entities(compressed: &[u8], min_y: i32, height: i32) -> io::Result<Box<[u8]>> {
        let data = zstd::decode_all(compressed).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("zstd decode failed: {error}"),
            )
        })?;
        let mut persistent: PersistentChunk<'_> = wincode::deserialize(&data).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("chunk decode failed: {error}"),
            )
        })?;
        ChunkStorage::validate_persistent_chunk(&persistent, min_y, height)?;

        if persistent.entities.is_empty() {
            return Ok(compressed.into());
        }
        persistent.entities.clear();
        let encoded = wincode::serialize(&persistent)
            .map_err(|error| io::Error::other(format!("chunk re-encode failed: {error}")))?;
        Ok(zstd::encode_all(encoded.as_slice(), CHUNK_COMPRESSION_LEVEL)?.into())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        path::PathBuf,
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    use steel_utils::Identifier;

    use super::*;
    use crate::chunk_saver::{
        PersistentBiomeData, PersistentBlockEntity, PersistentBlockState, PersistentEntity,
        PersistentLightData, PersistentSection,
        format::{ChunkEntry, FIRST_DATA_SECTOR},
    };

    const TEST_MIN_Y: i32 = 0;
    const TEST_HEIGHT: i32 = 16;
    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    fn test_directory(name: &str) -> PathBuf {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!(
            "steel-region-reader-{name}-{}-{sequence}",
            process::id()
        ))
    }

    fn test_thread_pool() -> rayon::ThreadPool {
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("test thread pool should build")
    }

    fn test_entity() -> PersistentEntity {
        PersistentEntity {
            entity_type: Identifier::vanilla_static("end_crystal"),
            uuid: [9; 16],
            pos: [1.0, 2.0, 3.0],
            motion: [0.0; 3],
            rotation: [0.0; 2],
            fall_distance: 0.0,
            remaining_fire_ticks: 0,
            ticks_frozen: 0,
            is_in_powder_snow: false,
            was_in_powder_snow: false,
            has_visual_fire: false,
            on_ground: false,
            no_gravity: false,
            invulnerable: false,
            air_supply: 0,
            portal_cooldown: 0,
            custom_name_nbt: Vec::new(),
            custom_name_visible: false,
            silent: false,
            glowing: false,
            tags: Vec::new(),
            custom_data_nbt: Vec::new(),
            nbt_data: Vec::new(),
            passengers: Vec::new(),
        }
    }

    fn test_chunk(entities: Vec<PersistentEntity>) -> PersistentChunk<'static> {
        PersistentChunk {
            block_attachments: Vec::new(),
            last_modified: 7,
            inhabited_time: 0,
            block_states: vec![PersistentBlockState {
                name: Identifier::vanilla_static("stone"),
                properties: Vec::new(),
            }],
            biomes: vec![Identifier::vanilla_static("plains")],
            sections: vec![PersistentSection::Homogeneous {
                block_state: 0,
                biomes: PersistentBiomeData::Homogeneous { biome: 0 },
            }],
            block_entities: vec![PersistentBlockEntity {
                x: 1,
                y: 5,
                z: 2,
                entity_type: Some(Identifier::vanilla_static("chest")),
                nbt_data: vec![1, 2, 3],
            }],
            entities,
            block_ticks: Vec::new(),
            fluid_ticks: Vec::new(),
            heightmaps: Vec::new(),
            light: PersistentLightData::default(),
            postprocessing: Vec::new(),
            structure_starts: Vec::new(),
            structure_references: Vec::new(),
            pois: Vec::new(),
        }
    }

    fn compress(persistent: &PersistentChunk<'_>) -> Vec<u8> {
        let encoded = wincode::serialize(persistent).expect("test chunk should encode");
        zstd::encode_all(encoded.as_slice(), CHUNK_COMPRESSION_LEVEL)
            .expect("test chunk should compress")
    }

    /// Writes `r.<x>.<z>.srg` holding one chunk per `(chunk_pos, payload)`, one sector run each.
    fn write_region(directory: &Path, pos: RegionPos, chunks: &[(ChunkPos, Vec<u8>)]) {
        std::fs::create_dir_all(directory).expect("test directory should be created");
        let mut header = RegionHeader::new();
        let mut body = Vec::new();
        let mut next_sector = FIRST_DATA_SECTOR;
        for (chunk_pos, payload) in chunks {
            let (local_x, local_z) = RegionPos::local_chunk_pos(chunk_pos.0.x, chunk_pos.0.y);
            let entry = ChunkEntry::new(next_sector, payload.len() as u32, ChunkStatus::Full);
            header.entries[RegionHeader::chunk_index(local_x, local_z)] = entry;
            body.extend_from_slice(payload);
            body.resize(body.len().next_multiple_of(SECTOR_SIZE), 0);
            next_sector += entry.sector_count();
        }

        let mut file = vec![0u8; FILE_HEADER_SIZE];
        file[0..4].copy_from_slice(&REGION_MAGIC);
        file[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        file.extend_from_slice(&header.to_bytes());
        file.resize(FIRST_DATA_SECTOR as usize * SECTOR_SIZE, 0);
        file.extend_from_slice(&body);
        std::fs::write(directory.join(pos.filename()), file)
            .expect("test region should be written");
    }

    fn snapshot(directory: &Path) -> Vec<(String, Vec<u8>, std::time::SystemTime)> {
        let mut files: Vec<_> = std::fs::read_dir(directory)
            .expect("test directory should be readable")
            .map(|entry| {
                let entry = entry.expect("directory entry should be readable");
                let modified = entry.metadata().and_then(|m| m.modified()).expect("mtime");
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).expect("file should be readable"),
                    modified,
                )
            })
            .collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }

    async fn read(directory: &Path) -> io::Result<Vec<RegionChunk>> {
        RegionReader::read_all(directory, TEST_MIN_Y, TEST_HEIGHT, &test_thread_pool()).await
    }

    #[tokio::test]
    async fn entities_are_removed_and_everything_else_is_kept() {
        let directory = test_directory("entities");
        let with_entities = ChunkPos::new(-1, -1);
        let without_entities = ChunkPos::new(1, 2);
        write_region(
            &directory,
            RegionPos::new(-1, -1),
            &[(with_entities, compress(&test_chunk(vec![test_entity()])))],
        );
        let untouched = compress(&test_chunk(Vec::new()));
        write_region(
            &directory,
            RegionPos::new(0, 0),
            &[(without_entities, untouched.clone())],
        );
        std::fs::write(directory.join("notes.txt"), b"ignored").expect("note should be written");
        let before = snapshot(&directory);

        let chunks = read(&directory).await.expect("template region should read");

        assert_eq!(snapshot(&directory), before);
        assert_eq!(chunks.len(), 2);
        let (pos, status, payload) = &chunks[0];
        assert_eq!((*pos, *status), (with_entities, ChunkStatus::Full));
        let data = zstd::decode_all(&payload[..]).expect("payload should be zstd");
        let persistent: PersistentChunk<'_> =
            wincode::deserialize(&data).expect("payload should decode");
        assert!(persistent.entities.is_empty());
        assert_eq!(persistent.block_entities.len(), 1);
        assert_eq!(persistent.block_entities[0].nbt_data, [1, 2, 3]);
        assert_eq!(persistent.block_states.len(), 1);
        assert_eq!(persistent.biomes.len(), 1);
        assert_eq!(persistent.sections.len(), 1);
        // A payload without entities is returned untouched.
        assert_eq!(chunks[1].0, without_entities);
        assert_eq!(&chunks[1].2[..], &untouched[..]);
        std::fs::remove_dir_all(directory).expect("test directory should be removable");
    }

    #[tokio::test]
    async fn wrong_magic_or_version_fails_without_touching_the_file() {
        for (name, offset, byte) in [("magic", 0, b'X'), ("version", 4, 0xFE)] {
            let directory = test_directory(name);
            let pos = ChunkPos::new(0, 0);
            write_region(
                &directory,
                RegionPos::new(0, 0),
                &[(pos, compress(&test_chunk(Vec::new())))],
            );
            let path = directory.join(RegionPos::new(0, 0).filename());
            let mut bytes = std::fs::read(&path).expect("region should be readable");
            bytes[offset] = byte;
            std::fs::write(&path, bytes).expect("region should be rewritten");
            let before = snapshot(&directory);

            let error = read(&directory).await.expect_err("bad header should fail");

            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("r.0.0.srg"), "{error}");
            assert_eq!(snapshot(&directory), before, "{name}");
            std::fs::remove_dir_all(directory).expect("test directory should be removable");
        }
    }

    #[tokio::test]
    async fn corrupt_chunk_fails_the_read_naming_region_and_chunk() {
        let directory = test_directory("corrupt");
        let good = ChunkPos::new(0, 0);
        let bad = ChunkPos::new(1, 2);
        write_region(
            &directory,
            RegionPos::new(0, 0),
            &[
                (good, compress(&test_chunk(Vec::new()))),
                (bad, b"this is not a zstd frame".to_vec()),
            ],
        );
        let before = snapshot(&directory);

        let error = read(&directory)
            .await
            .expect_err("corrupt chunk should fail");

        let message = error.to_string();
        assert!(message.contains("r.0.0.srg"), "{message}");
        assert!(message.contains("chunk (1, 2)"), "{message}");
        assert_eq!(snapshot(&directory), before);
        std::fs::remove_dir_all(directory).expect("test directory should be removable");
    }

    #[tokio::test]
    async fn missing_region_directory_is_empty_and_not_created() {
        let directory = test_directory("missing");

        let chunks = read(&directory)
            .await
            .expect("missing directory is an empty world");

        assert!(chunks.is_empty());
        assert!(!directory.exists());
    }
}
