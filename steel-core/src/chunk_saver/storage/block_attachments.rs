//! Validate generic block claims before materializing a saved chunk.

use super::{ChunkStorage, io};
use crate::{chunk::block_attachment::BlockAttachment, chunk_saver::format::PersistentChunk};
use rustc_hash::FxHashMap;
use std::sync::Arc;
use steel_utils::{BlockPos, ChunkPos};

impl ChunkStorage {
    pub(super) fn block_attachments_from_persistent(
        persistent: &PersistentChunk<'_>,
        chunk: ChunkPos,
        min_y: i32,
        height: i32,
    ) -> io::Result<FxHashMap<BlockPos, Arc<BlockAttachment>>> {
        let mut records = FxHashMap::default();
        for entry in &persistent.block_attachments {
            if entry.x >= 16 || entry.z >= 16 || entry.y < min_y || entry.y >= min_y + height {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "block attachment is outside its chunk",
                ));
            }
            let states = entry
                .states
                .iter()
                .map(|index| Self::resolve_block_state(persistent, *index))
                .collect::<io::Result<Vec<_>>>()?;
            let pos = BlockPos::new(
                chunk.0.x * 16 + i32::from(entry.x),
                entry.y,
                chunk.0.y * 16 + i32::from(entry.z),
            );
            if records
                .insert(
                    pos,
                    Arc::new(BlockAttachment {
                        key: entry.key.clone(),
                        states,
                        data: entry.data.clone(),
                    }),
                )
                .is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate block attachment position",
                ));
            }
        }
        Ok(records)
    }
}
