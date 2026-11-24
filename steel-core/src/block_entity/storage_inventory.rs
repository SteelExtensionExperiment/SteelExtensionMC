//! Native, component-preserving storage for extension blocks.

use std::{
    mem,
    num::NonZeroU32,
    sync::{Arc, Weak},
};

use simdnbt::{
    borrow::{BaseNbtCompound, NbtCompound as NbtView},
    owned::{NbtCompound, NbtList, NbtTag},
};
use smallvec::SmallVec;
use steel_registry::{
    block_entity_type::BlockEntityTypeRef, client_content::BlockItemDisplay, item_stack::ItemStack,
};
use steel_utils::{
    BlockPos, BlockStateId, ChunkPos, DowncastType, DowncastTypeKey,
    locks::{IntoShared, Shared, SyncMutex},
};

use super::{BlockEntity, BlockEntityBase};
use crate::{
    chunk::chunk_holder::ChunkHolder,
    inventory::{
        container::Container,
        lock::{ContainerRef, SharedContainer},
    },
    world::World,
};

const ITEMS_TAG: &str = "Items";
const SLOT_TAG: &str = "Slot";

/// Persistent inventory for a server-only block entity.
pub struct StorageBlockEntity {
    base: Arc<BlockEntityBase>,
    inventory: Shared<StorageInventory>,
    capacity: NonZeroU32,
    appearance: SyncMutex<StorageAppearance>,
}

/// Client decoration anchored to a storage owner.
#[derive(Debug)]
pub struct StorageDisplay {
    /// Display model attached to the storage owner.
    pub model: BlockItemDisplay,
    /// Construction positions covered by this decoration while the owner is visible.
    pub parts: Box<[BlockPos]>,
}

#[derive(Default)]
struct StorageAppearance {
    display: Option<Arc<StorageDisplay>>,
    chunks: SmallVec<[Option<ChunkRevision>; 4]>,
}

struct ChunkRevision {
    holder: Weak<ChunkHolder>,
    revision: u64,
}

impl PartialEq for ChunkRevision {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision && self.holder.ptr_eq(&other.holder)
    }
}

#[derive(Default)]
struct StorageInventory(Vec<ItemStack>);

// SAFETY: Both keys identify distinct concrete Steel implementations.
unsafe impl DowncastType for StorageBlockEntity {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:block_entity/storage_inventory");
}

// SAFETY: This key identifies the separately locked storage inventory.
unsafe impl DowncastType for StorageInventory {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:container/storage_inventory");
}

impl Container for StorageInventory {
    fn items(&self) -> &[ItemStack] {
        &self.0
    }

    fn items_mut(&mut self) -> &mut [ItemStack] {
        &mut self.0
    }

    fn set_changed(&mut self) {}
}

impl StorageBlockEntity {
    pub(super) fn new(
        kind: BlockEntityTypeRef,
        capacity: NonZeroU32,
        level: Weak<World>,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Self {
        Self {
            base: Arc::new(BlockEntityBase::new(kind, level, pos, state)),
            inventory: StorageInventory::default().into_shared(),
            capacity,
            appearance: SyncMutex::new(StorageAppearance::default()),
        }
    }

    /// Checks whether a chunk in the resolver's horizontal neighborhood changed.
    /// Unloaded chunks participate in the comparison without being requested.
    #[must_use]
    pub fn appearance_needs_refresh(&self, world: &World, radius: u8) -> bool {
        let pos = self.get_block_pos();
        let radius = i32::from(radius);
        let mut chunks = SmallVec::new();
        for x in (pos.x() - radius).div_euclid(16)..=(pos.x() + radius).div_euclid(16) {
            for z in (pos.z() - radius).div_euclid(16)..=(pos.z() + radius).div_euclid(16) {
                chunks.push(
                    world
                        .chunk_map
                        .active_full_chunk_holder(ChunkPos::new(x, z))
                        .map(|holder| ChunkRevision {
                            holder: Arc::downgrade(&holder),
                            revision: holder.packet_content_revision(),
                        }),
                );
            }
        }
        let mut appearance = self.appearance.lock();
        if appearance.chunks == chunks {
            return false;
        }
        appearance.chunks = chunks;
        true
    }

    /// Makes the requested range available while retaining the backing inventory
    /// for other viewers. A smaller enclosure cannot hide occupied slots.
    pub fn prepare_inventory(&self, slots: usize) -> Result<ContainerRef, &'static str> {
        if slots == 0 || slots > self.capacity.get() as usize {
            return Err("storage capacity is out of range");
        }
        let mut inventory = self.inventory.lock();
        if inventory.0.len() < slots {
            inventory.0.resize_with(slots, ItemStack::empty);
        } else if inventory.0[slots..].iter().any(|item| !item.is_empty()) {
            return Err("Rebuild a larger shell to access the remaining items");
        }
        drop(inventory);
        let inventory = Arc::clone(&self.inventory);
        let shared: SharedContainer = inventory;
        Ok(ContainerRef::owned_by_block_entity(
            shared,
            Arc::clone(&self.base),
        ))
    }

    /// Updates the decorative model for the storage block.
    /// The block update also invalidates any pending encoded chunk snapshot.
    pub fn set_display(&self, world: &World, display: Option<StorageDisplay>) {
        let mut appearance = self.appearance.lock();
        let unchanged = match (appearance.display.as_deref(), display.as_ref()) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.parts == b.parts
                    && a.model.translation == b.model.translation
                    && a.model.scale == b.model.scale
                    && a.model.rotation == b.model.rotation
                    && a.model.pack_transform == b.model.pack_transform
                    && ItemStack::is_same_item_same_components(&a.model.item, &b.model.item)
            }
            _ => false,
        };
        if unchanged {
            return;
        }
        appearance.display = display.map(Arc::new);
        drop(appearance);
        world.send_block_updated(self.get_block_pos());
    }

    #[must_use]
    pub(crate) fn display(&self) -> Option<Arc<StorageDisplay>> {
        self.appearance.lock().display.as_ref().map(Arc::clone)
    }
}

impl BlockEntity for StorageBlockEntity {
    fn base(&self) -> &BlockEntityBase {
        &self.base
    }

    fn load_additional(&self, nbt: &BaseNbtCompound<'_>) {
        let view: NbtView<'_, '_> = nbt.into();
        let mut inventory = self.inventory.lock();
        inventory.0.fill(ItemStack::empty());
        let Some(entries) = view.list(ITEMS_TAG).and_then(|list| list.compounds()) else {
            return;
        };
        for entry in entries {
            let Some(slot) = entry
                .int(SLOT_TAG)
                .and_then(|value| usize::try_from(value).ok())
                .filter(|&slot| slot < self.capacity.get() as usize)
            else {
                log::warn!("Invalid storage slot at {:?}", self.get_block_pos());
                continue;
            };
            let Some(item) = ItemStack::from_borrowed_compound(&entry) else {
                log::warn!(
                    "Invalid stored item at {:?}, slot {slot}",
                    self.get_block_pos()
                );
                continue;
            };
            if inventory.0.len() <= slot {
                inventory.0.resize_with(slot + 1, ItemStack::empty);
            }
            inventory.0[slot] = item;
        }
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        let inventory = self.inventory.lock();
        let items = inventory
            .items()
            .iter()
            .enumerate()
            .filter_map(|(slot, item)| {
                if item.is_empty() {
                    return None;
                }
                let NbtTag::Compound(mut item) = item.to_nbt_tag_ref() else {
                    return None;
                };
                item.insert(SLOT_TAG, slot as i32);
                Some(item)
            })
            .collect();
        nbt.insert(ITEMS_TAG, NbtList::Compound(items));
    }

    fn pre_remove_side_effects(&self, pos: BlockPos, _state: BlockStateId) {
        let Some(world) = self.get_level() else {
            return;
        };
        // Keep slot indices valid until menus retaining this inventory close.
        let items: Vec<_> = self
            .inventory
            .lock()
            .items_mut()
            .iter_mut()
            .map(mem::take)
            .collect();
        for item in items {
            world.drop_item_stack(pos, item);
        }
    }
}
