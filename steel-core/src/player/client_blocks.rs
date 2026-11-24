//! Per-connection visual attachments for projected server blocks.
//!
//! These are client-only displays, not world entities. Their lifetime is the
//! client's chunk membership, owned under the same lock as chunk packet commits.

use glam::{DVec3, Vec3};
use rustc_hash::{FxHashMap, FxHashSet};
use std::{collections::hash_map::Entry, mem, sync::Arc};
use steel_protocol::packets::game::{CAddEntity, CRemoveEntities, CSetEntityData};
use steel_protocol::{packet_traits::EncodedPacket, utils::ConnectionProtocol};
use steel_registry::{
    REGISTRY, RegistryEntry, blocks::block_state_ext::BlockStateExt, vanilla_entities,
    vanilla_entity_data::ItemDisplayEntityData,
};
use steel_utils::{BlockPos, BlockStateId, ChunkPos};
use uuid::Uuid;

use super::{
    PlayerConnection,
    connection::{BundleBuilder, NetworkConnection},
};
use crate::{
    block_entity::StorageDisplay, chunk::full_chunk::FullChunkRef, entity::next_entity_id,
    world::World,
};

#[derive(Debug)]
struct DisplayGroup {
    state: BlockStateId,
    storage_display: Option<Arc<StorageDisplay>>,
    ids: Vec<i32>,
}

#[derive(Debug, Clone)]
pub(crate) struct BlockVisual {
    pub(crate) pos: BlockPos,
    pub(crate) state: BlockStateId,
    storage_display: Option<Arc<StorageDisplay>>,
}

impl BlockVisual {
    pub(crate) fn live(world: &World, pos: BlockPos, state: BlockStateId) -> Self {
        let storage_display = if REGISTRY
            .block_entity_types
            .is_storage_block(state.get_block())
        {
            world
                .chunk_map
                .with_full_chunk(ChunkPos::from_block_pos(pos), |chunk| {
                    chunk.storage_display(pos, state)
                })
                .flatten()
        } else {
            None
        };
        Self {
            pos,
            state,
            storage_display,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ClientBlockDisplays {
    chunks: FxHashMap<ChunkPos, FxHashMap<BlockPos, DisplayGroup>>,
    // A construction part can be covered by more than one visible assembly.
    covered: FxHashMap<BlockPos, usize>,
}

impl ClientBlockDisplays {
    /// The snapshot is collected in the same revision window as the chunk packet.
    pub(crate) fn snapshot(chunk: FullChunkRef<'_>) -> Vec<BlockVisual> {
        if !REGISTRY.client_content.has_blocks() {
            return Vec::new();
        }
        let mut blocks = Vec::new();
        let chunk_pos = chunk.common().pos();
        for (section_index, holder) in chunk.sections().sections.iter().enumerate() {
            let section = holder.read();
            let states = section.states();
            if !states.maybe_has(|state| REGISTRY.client_content.block(state.get_block()).is_some())
            {
                continue;
            }
            for index in 0..16 * 16 * 16 {
                let state = states.get_at_index(index);
                if REGISTRY.client_content.block(state.get_block()).is_none() {
                    continue;
                }
                let pos = BlockPos::new(
                    chunk_pos.0.x * 16 + (index & 15) as i32,
                    chunk.min_y() + section_index as i32 * 16 + (index >> 8) as i32,
                    chunk_pos.0.y * 16 + ((index >> 4) & 15) as i32,
                );
                blocks.push(BlockVisual {
                    pos,
                    state,
                    storage_display: chunk.storage_display(pos, state),
                });
            }
        }
        blocks
    }

    pub(crate) fn replace_chunk(
        &mut self,
        connection: &PlayerConnection,
        chunk: ChunkPos,
        blocks: &[BlockVisual],
    ) {
        let mut changed = FxHashSet::default();
        self.forget_chunk(connection, chunk, &mut changed);
        if connection.closed() {
            return;
        }
        for visual in blocks {
            self.update_source(connection, visual, &mut changed);
        }
        for pos in changed {
            self.refresh_display(connection, pos);
        }
    }

    pub(crate) fn update(&mut self, connection: &PlayerConnection, visual: &BlockVisual) {
        if connection.closed() {
            return;
        }
        let mut changed = FxHashSet::default();
        self.update_source(connection, visual, &mut changed);
        let source_changed = changed.remove(&visual.pos);
        for pos in changed {
            self.refresh_display(connection, pos);
        }
        if source_changed {
            self.refresh_display(connection, visual.pos);
        }
    }

    fn update_source(
        &mut self,
        connection: &PlayerConnection,
        visual: &BlockVisual,
        changed: &mut FxHashSet<BlockPos>,
    ) {
        let BlockVisual {
            pos,
            state,
            storage_display,
        } = visual;
        let (pos, state) = (*pos, *state);
        let chunk_pos = ChunkPos::from_block_pos(pos);
        if let Some(group) = self
            .chunks
            .get(&chunk_pos)
            .and_then(|blocks| blocks.get(&pos))
            && group.state == state
            && match (&group.storage_display, storage_display) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
        {
            return;
        }
        let old = self
            .chunks
            .get_mut(&chunk_pos)
            .and_then(|blocks| blocks.remove(&pos));
        if let Some(old) = old {
            if connection.uses_resource_pack()
                && let Some(display) = old.storage_display
            {
                self.change_coverage(&display.parts, false, changed);
            }
            if !old.ids.is_empty() {
                Self::send_removals(connection, old.ids);
            }
        }
        if let Some(display) = storage_display {
            if connection.uses_resource_pack() {
                self.change_coverage(&display.parts, true, changed);
            }
        } else if !Self::has_registered_displays(connection, state) {
            if self.chunks.get(&chunk_pos).is_some_and(FxHashMap::is_empty) {
                self.chunks.remove(&chunk_pos);
            }
            return;
        }
        self.chunks.entry(chunk_pos).or_default().insert(
            pos,
            DisplayGroup {
                state,
                storage_display: storage_display.as_ref().map(Arc::clone),
                ids: Vec::new(),
            },
        );
        changed.insert(pos);
    }

    fn has_registered_displays(connection: &PlayerConnection, state: BlockStateId) -> bool {
        let block = state.get_block();
        let content = &REGISTRY.client_content;
        (connection.uses_resource_pack() || !content.is_construction_part(block))
            && content
                .block(block)
                .is_some_and(|model| !model.displays_for(state).is_empty())
    }

    fn change_coverage(
        &mut self,
        parts: &[BlockPos],
        visible: bool,
        changed: &mut FxHashSet<BlockPos>,
    ) {
        for &part in parts {
            match (self.covered.entry(part), visible) {
                (Entry::Vacant(entry), true) => {
                    entry.insert(1);
                    changed.insert(part);
                }
                (Entry::Occupied(mut entry), true) => *entry.get_mut() += 1,
                (Entry::Occupied(mut entry), false) => {
                    if *entry.get() == 1 {
                        entry.remove();
                        changed.insert(part);
                    } else {
                        *entry.get_mut() -= 1;
                    }
                }
                (Entry::Vacant(_), false) => {}
            }
        }
    }

    fn refresh_display(&mut self, connection: &PlayerConnection, pos: BlockPos) {
        let Some(group) = self
            .chunks
            .get_mut(&ChunkPos::from_block_pos(pos))
            .and_then(|blocks| blocks.get_mut(&pos))
        else {
            return;
        };
        let content = &REGISTRY.client_content;
        let block = group.state.get_block();
        let displays = if let Some(display) = &group.storage_display {
            std::slice::from_ref(&display.model)
        } else if content.is_construction_part(block) && self.covered.contains_key(&pos) {
            &[]
        } else {
            content
                .block(block)
                .map_or(&[][..], |model| model.displays_for(group.state))
        };
        if displays.is_empty() {
            if !group.ids.is_empty() {
                Self::send_removals(connection, mem::take(&mut group.ids));
            }
            return;
        }
        if !group.ids.is_empty() {
            return;
        }

        group.ids.reserve(displays.len());
        let mut bundle = BundleBuilder::new(connection.compression());
        let uses_pack = connection.uses_resource_pack();
        for display in displays {
            let (item, transform) = display.for_client(uses_pack);
            let id = next_entity_id();
            let mut data = ItemDisplayEntityData::new();
            data.item_stack.set(item);
            // Vanilla probes display light at its entity position. Keep that
            // probe above the opaque carrier and compensate the model origin.
            let probe_y = if group.storage_display.is_some() {
                (transform.translation.y + transform.scale.y * 0.5).ceil()
            } else {
                1.0
            };
            let anchor = Vec3::new(0.5, probe_y, 0.5);
            data.display.translation.set(transform.translation - anchor);
            data.display.scale.set(transform.scale);
            data.display.left_rotation.set(transform.rotation);
            data.display.base.no_gravity.set(true);
            bundle.add(CAddEntity {
                id,
                uuid: Uuid::new_v4(),
                entity_type: vanilla_entities::ITEM_DISPLAY.id() as i32,
                position: DVec3::new(f64::from(pos.x()), f64::from(pos.y()), f64::from(pos.z()))
                    + anchor.as_dvec3(),
                velocity: DVec3::ZERO,
                x_rot: 0,
                y_rot: 0,
                head_y_rot: 0,
                data: 0,
            });
            bundle.add(CSetEntityData::new(id, data.pack_all()));
            group.ids.push(id);
        }
        connection.send_encoded_bundle(bundle.into_packets());
    }

    pub(crate) fn remove_chunk(&mut self, connection: &PlayerConnection, chunk: ChunkPos) {
        let mut changed = FxHashSet::default();
        self.forget_chunk(connection, chunk, &mut changed);
        if !connection.closed() {
            for pos in changed {
                self.refresh_display(connection, pos);
            }
        }
    }

    fn forget_chunk(
        &mut self,
        connection: &PlayerConnection,
        chunk: ChunkPos,
        changed: &mut FxHashSet<BlockPos>,
    ) {
        let Some(blocks) = self.chunks.remove(&chunk) else {
            return;
        };
        let mut ids = Vec::new();
        for group in blocks.into_values() {
            if connection.uses_resource_pack()
                && let Some(display) = group.storage_display
            {
                self.change_coverage(&display.parts, false, changed);
            }
            ids.extend(group.ids);
        }
        if !ids.is_empty() && !connection.closed() {
            Self::send_removals(connection, ids);
        }
    }

    pub(crate) fn clear(&mut self, connection: &PlayerConnection) {
        let chunks = mem::take(&mut self.chunks);
        self.covered.clear();
        if connection.closed() || chunks.is_empty() {
            return;
        }
        let entity_ids: Vec<_> = chunks
            .into_values()
            .flat_map(FxHashMap::into_values)
            .flat_map(|group| group.ids)
            .collect();
        if !entity_ids.is_empty() {
            Self::send_removals(connection, entity_ids);
        }
    }

    fn send_removals(connection: &PlayerConnection, entity_ids: Vec<i32>) {
        match EncodedPacket::from_bare(
            CRemoveEntities { entity_ids },
            connection.compression(),
            ConnectionProtocol::Play,
        ) {
            Ok(packet) => connection.send_encoded(packet),
            Err(error) => log::warn!("failed to encode client block display removal: {error}"),
        }
    }
}
