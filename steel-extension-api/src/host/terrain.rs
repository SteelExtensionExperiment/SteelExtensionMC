//! Temporary terrain policy built on persistent block-state attachments.

use std::sync::Arc;
use steel_core::{
    chunk::block_attachment::BlockAttachment,
    extension::{ExtensionHooks, WorldTickHandler},
    world::{ConditionalBlockSetResult, World},
};
use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_utils::{BlockPos, BlockStateId, ChunkPos, Identifier, types::UpdateFlags};

const KEY: Identifier = Identifier::new_static("steel", "temporary_terrain");

struct TemporaryBlock {
    original: BlockStateId,
    placed: BlockStateId,
    expires_at: i64,
}
impl TemporaryBlock {
    fn read(attachment: &BlockAttachment) -> Result<Self, String> {
        let [original, placed] = attachment.states.as_slice() else {
            return Err("temporary terrain needs exactly two state references".into());
        };
        if original == placed || original.has_block_entity() || placed.has_block_entity() {
            return Err("invalid temporary terrain states".into());
        }
        let bytes: [u8; 8] = attachment
            .data
            .as_slice()
            .try_into()
            .map_err(|_| "invalid temporary terrain deadline")?;
        Ok(Self {
            original: *original,
            placed: *placed,
            expires_at: i64::from_le_bytes(bytes),
        })
    }
}

pub(crate) fn set(
    world: &Arc<World>,
    pos: BlockPos,
    expected: BlockStateId,
    placed: BlockStateId,
    duration: u32,
) -> Result<bool, String> {
    if duration == 0 || expected == placed {
        return Err("temporary terrain needs a positive duration and a different state".into());
    }
    if expected.has_block_entity() || placed.has_block_entity() {
        return Err("temporary terrain cannot replace or create block entities".into());
    }
    if !world.is_in_valid_bounds(pos) {
        return Err("temporary block is outside world bounds".into());
    }
    let expires_at = world
        .game_time()
        .checked_add(i64::from(duration))
        .ok_or("temporary block expiry overflow")?;
    let available = world
        .chunk_map
        .with_full_chunk(ChunkPos::from_block_pos(pos), |chunk| {
            chunk.block_attachment(pos).is_none()
        })
        .ok_or("temporary block chunk is not loaded")?;
    if !available {
        return Err("another attachment already owns this position".into());
    }
    let attachment = Arc::new(BlockAttachment {
        key: KEY,
        states: vec![expected, placed],
        data: expires_at.to_le_bytes().to_vec(),
    });
    match world.set_block_with_attachment(
        pos,
        expected,
        placed,
        UpdateFlags::UPDATE_ALL,
        attachment,
    ) {
        ConditionalBlockSetResult::Changed => Ok(true),
        ConditionalBlockSetResult::Unchanged | ConditionalBlockSetResult::Stale(_) => Ok(false),
        ConditionalBlockSetResult::Unavailable => {
            Err("temporary block chunk is unavailable".into())
        }
    }
}

/// Unloaded chunks retain their absolute deadlines until block ticking resumes.
fn restore_expired(world: &Arc<World>, now: i64) -> Result<(), String> {
    for chunk_pos in world.chunk_map.block_ticking_full_chunk_positions() {
        let attachments = world
            .chunk_map
            .with_full_chunk(chunk_pos, |chunk| chunk.block_attachments())
            .unwrap_or_default();
        for (pos, attachment) in attachments {
            if attachment.key != KEY {
                continue;
            }
            let record = TemporaryBlock::read(&attachment)?;
            if record.expires_at > now {
                continue;
            }
            if matches!(
                world.set_block_if_unchanged(
                    pos,
                    record.placed,
                    record.original,
                    UpdateFlags::UPDATE_ALL
                ),
                ConditionalBlockSetResult::Unavailable
            ) {
                continue;
            }
            world.chunk_map.with_full_chunk(chunk_pos, |chunk| {
                chunk.remove_block_attachment(pos, &attachment);
            });
        }
    }
    Ok(())
}

struct TerrainTick;
impl WorldTickHandler for TerrainTick {
    fn tick(&self, world: &Arc<World>, _tick_count: u64) {
        if let Err(error) = restore_expired(world, world.game_time()) {
            log::error!("temporary terrain in {}: {error}", world.key);
        }
    }
}

pub(crate) fn install(hooks: &mut ExtensionHooks) {
    hooks.register_world_block_tick(Box::new(TerrainTick));
}

#[cfg(test)]
mod tests;
