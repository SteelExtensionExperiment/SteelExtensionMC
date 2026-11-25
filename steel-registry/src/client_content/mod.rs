//! Server-only items and blocks projected into vanilla client representations.
//!
//! Registrations precede registry freezing. Persistent codecs retain real keys;
//! only network codecs use these mappings. Models are block-bound, client-only
//! item displays, rebuilt when a player receives a chunk.

mod bed;
mod biome;
mod definition;
mod entity;
mod fluid;
pub use biome::{ClientBiomes, TintVariant};
pub use entity::{ClientEntity, EntityRegistration, MobCarrier, MobStyle};
pub use fluid::{ClientFluid, FluidRegistration, FluidSpread, FluidTint};
mod horizontal;
pub use horizontal::{ClientHorizontal, HorizontalPlacement};
mod projection;
pub use definition::{BlockDefinition, PlacingItem};
mod registration;
mod resource_pack;
mod stack;

pub use bed::{BedModel, BedRegistration, ClientBed};

pub use registration::BlockItemRegistration;
pub use registration::on_registration;
pub(crate) use registration::run_registration_hooks;
pub use resource_pack::{PackHeadFallback, ResourcePackAssets, project_pack_models};
pub(crate) use stack::has_server_item;
pub use stack::{client_stack, client_template, restore_client_stack};

use glam::{Quat, Vec3};
use rustc_hash::{FxHashMap, FxHashSet};
use steel_utils::BlockStateId;

use crate::{
    REGISTRY, RegistryEntry,
    blocks::BlockRef,
    data_components::vanilla_components::{ITEM_MODEL, PROFILE},
    entity_type::EntityTypeRef,
    fluid::FluidRef,
    item_stack::ItemStack,
    items::ItemRef,
};

/// A stationary display relative to the placed block's minimum corner.
#[derive(Debug, Clone)]
pub struct BlockItemDisplay {
    pub item: ItemStack,
    pub translation: Vec3,
    pub scale: Vec3,
    pub rotation: Quat,
    pub pack_transform: Option<BlockDisplayTransform>,
}

impl BlockItemDisplay {
    /// Returns the display stack and placement for the recipient's pack status.
    #[must_use]
    pub fn for_client(&self, uses_pack: bool) -> (ItemStack, BlockDisplayTransform) {
        let transform = BlockDisplayTransform {
            translation: self.translation,
            scale: self.scale,
            rotation: self.rotation,
        };
        let packed = self.pack_transform.unwrap_or(transform);
        if uses_pack {
            return (self.item.clone(), packed);
        }
        let Some(fallback) = self
            .item
            .get(ITEM_MODEL)
            .and_then(|model| REGISTRY.client_content.resource_pack().head_fallback(model))
        else {
            return (self.item.clone(), transform);
        };
        let mut item = ItemStack::new(fallback.item);
        if let Some(profile) = &fallback.profile {
            item.set(PROFILE, profile.clone());
        }
        // A head is half a block wide, with its origin centered on its top face.
        let offset = Vec3::Y * (packed.scale.y * 0.5);
        (
            item,
            BlockDisplayTransform {
                translation: packed.translation + packed.rotation * offset,
                scale: packed.scale * 2.0,
                rotation: packed.rotation,
            },
        )
    }
}

/// Alternate placement for the same display with the resource pack loaded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockDisplayTransform {
    pub translation: Vec3,
    pub scale: Vec3,
    pub rotation: Quat,
}

impl BlockDisplayTransform {
    /// Whether the transform can be sent to a client display entity.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.translation.is_finite()
            && self.scale.is_finite()
            && self.scale.min_element() > 0.0
            && self.rotation.is_finite()
            && (self.rotation.length_squared() - 1.0).abs() <= 1.0e-4
    }
}

/// A custom block's selected vanilla geometry, mining material, and attachments.
#[derive(Debug)]
pub struct ClientBlock {
    pub block: BlockRef,
    pub carrier: BlockStateId,
    pub material: Option<BlockRef>,
    /// The client's carrier cannot predict mining against this material.
    pub server_mining: bool,
    /// A complete bed state family, including facing, occupancy, and head/foot.
    pub bed: Option<ClientBed>,
    /// State-specific client projections and models for horizontal facing.
    pub horizontal: Option<ClientHorizontal>,
    /// A vanilla block with an identical state layout; each state projects onto
    /// the template state with the same offset. Used by liquid blocks.
    pub state_family: Option<BlockRef>,
    pub displays: Vec<BlockItemDisplay>,
    /// Deterministic default drop, independent of the carrier's loot table.
    pub self_drop: Option<ItemRef>,
}

impl ClientBlock {
    #[must_use]
    pub fn carrier_state(&self, state: BlockStateId) -> BlockStateId {
        if let Some(template) = self.state_family {
            let offset = state.0 - REGISTRY.blocks.get_base_state_id(self.block).0;
            return BlockStateId(REGISTRY.blocks.get_base_state_id(template).0 + offset);
        }
        if let Some(horizontal) = &self.horizontal {
            let offset = usize::from(state.0 - REGISTRY.blocks.get_base_state_id(self.block).0);
            return horizontal.states[offset].carrier;
        }
        let Some(bed) = &self.bed else {
            return self.carrier;
        };
        let offset = state.0 - REGISTRY.blocks.get_base_state_id(self.block).0;
        BlockStateId(REGISTRY.blocks.get_base_state_id(bed.template).0 + offset)
    }

    #[must_use]
    pub fn displays_for(&self, state: BlockStateId) -> &[BlockItemDisplay] {
        if let Some(horizontal) = &self.horizontal {
            let offset = usize::from(state.0 - REGISTRY.blocks.get_base_state_id(self.block).0);
            return &horizontal.states[offset].displays;
        }
        let Some(bed) = &self.bed else {
            return &self.displays;
        };
        let offset = usize::from(state.0 - REGISTRY.blocks.get_base_state_id(self.block).0);
        &bed.models[offset]
    }
}

#[derive(Default)]
pub struct ClientContent {
    resource_pack: ResourcePackAssets,
    items: FxHashMap<usize, ItemRef>,
    blocks: FxHashMap<usize, ClientBlock>,
    construction_parts: FxHashSet<usize>,
    /// Keyed by both the source and the flowing fluid id.
    fluids: FxHashMap<usize, &'static ClientFluid>,
    /// Each registered pair once, in registration order.
    fluid_entries: Vec<&'static ClientFluid>,
    /// Registered mob types, in registration order.
    entities: Vec<&'static ClientEntity>,
    tinting_fluid_count: u16,
    biomes: ClientBiomes,
    vanilla_item_count: usize,
    vanilla_block_count: usize,
    vanilla_fluid_count: usize,
    vanilla_state_count: usize,
    frozen: bool,
    projections: Vec<projection::ProjectionCandidate>,
}

impl ClientContent {
    /// Gets the resource pack assets contributed during content registration.
    #[must_use]
    pub const fn resource_pack(&self) -> &ResourcePackAssets {
        &self.resource_pack
    }

    pub(crate) const fn capture_vanilla(
        &mut self,
        items: usize,
        blocks: usize,
        states: usize,
        fluids: usize,
    ) {
        self.vanilla_item_count = items;
        self.vanilla_block_count = blocks;
        self.vanilla_fluid_count = fluids;
        self.vanilla_state_count = states;
    }

    pub(crate) const fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Builds the client biome list from the final server biome registry.
    pub(crate) fn build_client_biomes(&mut self, biomes: &crate::biome::BiomeRegistry) {
        self.biomes = ClientBiomes::build(biomes, &self.fluid_entries);
    }

    /// The biome list clients receive, including tint variants.
    #[must_use]
    pub const fn client_biomes(&self) -> &ClientBiomes {
        &self.biomes
    }

    #[must_use]
    pub fn item(&self, item: ItemRef) -> ItemRef {
        self.items.get(&item.id()).copied().unwrap_or(item)
    }

    #[must_use]
    pub fn is_custom_item(&self, item: ItemRef) -> bool {
        self.items.contains_key(&item.id())
    }

    #[must_use]
    pub fn has_items(&self) -> bool {
        !self.items.is_empty()
    }

    #[must_use]
    pub fn is_vanilla_item(&self, item: ItemRef) -> bool {
        item.try_id().is_some_and(|id| id < self.vanilla_item_count)
    }

    /// Vanilla fluids are the only ones clients know; custom fluids never leave the server.
    #[must_use]
    pub fn is_vanilla_fluid(&self, fluid: FluidRef) -> bool {
        fluid
            .try_id()
            .is_some_and(|id| id < self.vanilla_fluid_count)
    }

    #[must_use]
    pub fn is_vanilla_block(&self, block: BlockRef) -> bool {
        block
            .try_id()
            .is_some_and(|id| id < self.vanilla_block_count)
    }

    #[must_use]
    pub const fn vanilla_state_count(&self) -> usize {
        self.vanilla_state_count
    }

    #[must_use]
    pub fn block(&self, block: BlockRef) -> Option<&ClientBlock> {
        self.blocks.get(&block.id())
    }

    /// Whether the ordinary displays decorate an incomplete multiblock for packed clients.
    #[must_use]
    pub fn is_construction_part(&self, block: BlockRef) -> bool {
        self.construction_parts.contains(&block.id())
    }

    #[must_use]
    pub fn has_blocks(&self) -> bool {
        !self.blocks.is_empty()
    }

    /// The registered pair that owns a fluid variant, if it is a custom fluid.
    #[must_use]
    pub fn fluid(&self, fluid: FluidRef) -> Option<&'static ClientFluid> {
        fluid.try_id().and_then(|id| self.fluids.get(&id)).copied()
    }

    /// Every registered fluid pair, in registration order.
    #[must_use]
    pub fn fluids(&self) -> &[&'static ClientFluid] {
        &self.fluid_entries
    }

    /// The tint index of a fluid variant, if it belongs to a tinting fluid.
    ///
    /// This is on the block-set path, so it scans the few registered pairs by
    /// pointer instead of resolving registry ids.
    #[must_use]
    pub fn tint_index_of(&self, fluid: FluidRef) -> Option<u16> {
        if self.tinting_fluid_count == 0 {
            return None;
        }
        self.fluid_entries
            .iter()
            .find(|entry| std::ptr::eq(entry.source, fluid) || std::ptr::eq(entry.flowing, fluid))
            .and_then(|entry| entry.tint_index)
    }

    /// The registered mob type behind an entity type, if it is one.
    #[must_use]
    pub fn entity(&self, entity_type: EntityTypeRef) -> Option<&'static ClientEntity> {
        self.entities
            .iter()
            .find(|entry| std::ptr::eq(entry.entity_type, entity_type))
            .copied()
    }

    /// Every registered mob type, in registration order.
    #[must_use]
    pub fn entities(&self) -> &[&'static ClientEntity] {
        &self.entities
    }

    /// Vanilla entity types are the only ones clients know.
    #[must_use]
    pub fn is_vanilla_entity_type(&self, entity_type: EntityTypeRef) -> bool {
        self.entity(entity_type).is_none()
    }

    /// The number of registered fluids that tint water.
    #[must_use]
    pub const fn tinting_fluid_count(&self) -> u16 {
        self.tinting_fluid_count
    }
}

/// The vanilla entity type clients see for an entity type.
#[must_use]
pub fn client_entity_type(entity_type: EntityTypeRef) -> EntityTypeRef {
    if !REGISTRY.is_initialized() {
        return entity_type;
    }
    REGISTRY
        .client_content
        .entity(entity_type)
        .map_or(entity_type, |entry| entry.carrier.entity_type())
}

/// The vanilla fluid clients see for a fluid. Vanilla fluids are returned unchanged.
///
/// Entity movement uses this rather than fluid tags: the client predicts its own
/// motion from the fluid it was sent, so the server must simulate the same one.
#[must_use]
pub fn client_fluid(fluid: FluidRef) -> FluidRef {
    if !REGISTRY.is_initialized() || REGISTRY.client_content.fluids.is_empty() {
        return fluid;
    }
    REGISTRY
        .client_content
        .fluid(fluid)
        .map_or(fluid, |entry| entry.client_variant(fluid))
}

/// Projects a runtime state at a client boundary, preserving vanilla states exactly.
#[must_use]
pub fn client_block_state(state: BlockStateId) -> BlockStateId {
    if !REGISTRY.is_initialized() || !REGISTRY.client_content.has_blocks() {
        return state;
    }
    REGISTRY
        .blocks
        .by_state_id(state)
        .and_then(|block| REGISTRY.client_content.block(block))
        .map_or(state, |entry| entry.carrier_state(state))
}

/// Projects a block registry entry for block events and other raw-ID payloads.
#[must_use]
pub fn client_block(block: BlockRef) -> BlockRef {
    REGISTRY.client_content.block(block).map_or(block, |entry| {
        // Carrier states were validated against the vanilla registry at registration.
        let Some(carrier) = REGISTRY.blocks.by_state_id(entry.carrier) else {
            panic!("registered client carrier state is missing");
        };
        carrier
    })
}
