//! Paged vanilla menu over the native inventory of a storage block.

pub(crate) mod display;

use std::sync::Arc;

use steel_core::{
    block_entity::{BlockEntityLifecycleExt as _, SharedBlockEntity, StorageBlockEntity},
    inventory::{container::DEFAULT_DISTANCE_BUFFER, prelude::*},
};
use steel_registry::{
    data_components::vanilla_components::CUSTOM_NAME, vanilla_items, vanilla_menu_types,
};
use steel_utils::{Downcast as _, DowncastType, DowncastTypeKey};
use text_components::TextComponent;

use crate::{
    BlockPos, World,
    menus::{DynStorageAccess, StorageAccessDyn as _},
};

const PAGE_SLOTS: usize = 45;
const FOOTER_SLOTS: usize = 9;
const PREVIOUS: usize = 0;
const NEXT: usize = FOOTER_SLOTS - 1;

pub(crate) fn open(
    player: &Player,
    core: BlockPos,
    interacted: BlockPos,
    title: &str,
    slots: u32,
    access: DynStorageAccess,
) -> Result<(), &'static str> {
    let world = player.get_world();
    let core = steel_utils::BlockPos::new(core.x, core.y, core.z);
    let interacted = steel_utils::BlockPos::new(interacted.x, interacted.y, interacted.z);
    if !in_reach(player, interacted) {
        return Err("Storage is out of reach");
    }
    let owner = world
        .get_block_entity(core)
        .ok_or("Storage core is unavailable")?;
    let storage = owner
        .downcast_ref::<StorageBlockEntity>()
        .ok_or("Block has no persistent inventory")?;
    let slots = slots as usize;
    if !access.is_valid(World::from_host(&world)) {
        return Err("Storage enclosure is incomplete");
    }
    let container = storage.prepare_inventory(slots)?;
    let view = Arc::new(StorageView {
        owner,
        interacted,
        container,
        access,
        slots,
        title: title.to_owned(),
    });
    view.open_page(player, 0);
    Ok(())
}

fn in_reach(player: &Player, pos: steel_utils::BlockPos) -> bool {
    player.is_within_block_interaction_range_with_buffer(pos, f64::from(DEFAULT_DISTANCE_BUFFER))
}

struct StorageView {
    owner: SharedBlockEntity,
    interacted: steel_utils::BlockPos,
    container: ContainerRef,
    access: DynStorageAccess,
    slots: usize,
    title: String,
}

impl StorageView {
    fn pages(&self) -> usize {
        self.slots.div_ceil(PAGE_SLOTS)
    }

    fn open_page(self: &Arc<Self>, player: &Player, page: usize) {
        let title = format!("{}  {}/{}", self.title, page + 1, self.pages());
        let view = Arc::clone(self);
        player.open_menu(TextComponent::plain(title), move |context| {
            let start = page * PAGE_SLOTS;
            let end = (start + PAGE_SLOTS).min(view.slots);
            let mut builder =
                MenuBuilder::new(&vanilla_menu_types::GENERIC_9X6, context.container_id);
            let storage = builder.section_at(&view.container, start..end, SectionKind::Normal);

            // The final page can have unused cells. Keep all controls at fixed positions.
            let padding = PAGE_SLOTS - (end - start);
            let mut icons = vec![ItemStack::empty(); padding + FOOTER_SLOTS];
            if page > 0 {
                icons[padding + PREVIOUS] = page_icon("Previous page");
            }
            if page + 1 < view.pages() {
                icons[padding + NEXT] = page_icon("Next page");
            }
            let controls = SimpleContainer::from_items(icons).into_shared();
            builder.section_all_with(controls, SectionKind::Display);
            let inventory = builder.player_inventory(&context.player.inventory);
            builder.route(storage, inventory.all(), FillDirection::Backward);
            builder.route(inventory.all(), storage, FillDirection::Forward);
            builder.build(Page { view, page })
        });
    }
}

fn page_icon(name: &str) -> ItemStack {
    let mut arrow = ItemStack::new(&vanilla_items::ARROW);
    arrow.set(CUSTOM_NAME, TextComponent::plain(name.to_owned()));
    arrow
}

struct Page {
    view: Arc<StorageView>,
    page: usize,
}

// SAFETY: This key uniquely identifies the extension host's paged inventory menu.
unsafe impl DowncastType for Page {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:extension/menu/paged_storage");
}

impl MenuKind for Page {
    fn still_valid(&self, _behavior: &MenuBehavior, player: &Player) -> bool {
        let world = player.get_world();
        !self.view.owner.is_removed()
            && in_reach(player, self.view.interacted)
            && world
                .get_block_entity(self.view.owner.get_block_pos())
                .is_some_and(|current| Arc::ptr_eq(&current, &self.view.owner))
            && self.view.access.is_valid(World::from_host(&world))
    }

    fn on_slot_clicked(
        &mut self,
        behavior: &mut MenuBehavior,
        guard: &mut ContainerLockGuard,
        click: Click,
        player: &Player,
    ) -> ClickOutcome {
        if self.view.owner.is_removed() {
            return ClickOutcome::Consume;
        }
        let Some(slot) = click.slot() else {
            return ClickOutcome::Fallthrough;
        };
        if !(PAGE_SLOTS..PAGE_SLOTS + FOOTER_SLOTS).contains(&slot) {
            return ClickOutcome::Fallthrough;
        }
        if matches!(click, Click::Pickup { .. }) && behavior.carried().is_empty() {
            let target = match slot - PAGE_SLOTS {
                PREVIOUS => self.page.checked_sub(1),
                NEXT if self.page + 1 < self.view.pages() => Some(self.page + 1),
                _ => None,
            };
            if let Some(page) = target {
                guard.run_unlocked(|| self.view.open_page(player, page));
            }
        }
        ClickOutcome::Consume
    }

    fn on_drag(
        &mut self,
        _behavior: &mut MenuBehavior,
        _guard: &mut ContainerLockGuard,
        _action: QuickCraft,
        _player: &Player,
    ) -> ClickOutcome {
        if self.view.owner.is_removed() {
            ClickOutcome::Consume
        } else {
            ClickOutcome::Fallthrough
        }
    }
}
