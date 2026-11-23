//! Lossless inventory snapshot: NBT encoding of slots 0-40, the selected slot and the game mode,
//! and the adapters behind `Server::inventory_snapshot` and `Server::restore_inventory`.
//!
//! Blob layout (nameless-root binary NBT, written with `simdnbt::owned::NbtTag::write`, read with
//! `simdnbt::borrow::read_tag`):
//! `{ Inventory: [{ Slot: byte, id, count, components? }, ...], SelectedItemSlot: int,
//! playerGameType: int }`. Entries are the persistent `ItemStack` codec plus `Slot`.
use super::{items::INVENTORY_SLOTS, players::find};
use crate::{
    PlayerId,
    inventory_snapshot::{INVENTORY_SNAPSHOT_FORMAT, InventorySnapshot},
};
use simdnbt::{
    borrow::read_tag,
    owned::{NbtCompound, NbtList, NbtTag},
};
use std::{array, io::Cursor, mem};
use steel_core::{
    inventory::container::Container, player::player_inventory::PlayerInventory,
    server::Server as CoreServer,
};
use steel_protocol::packets::game::CSetHeldSlot;
use steel_registry::item_stack::ItemStack;
use steel_utils::types::GameType;

const INVENTORY_KEY: &str = "Inventory";
const SELECTED_SLOT_KEY: &str = "SelectedItemSlot";
const GAME_TYPE_KEY: &str = "playerGameType";
const SLOT_KEY: &str = "Slot";

const GAME_TYPES: [GameType; 4] = [
    GameType::Survival,
    GameType::Creative,
    GameType::Adventure,
    GameType::Spectator,
];

/// Everything a snapshot holds, as host values. Used for capture and restore.
pub(crate) struct DecodedInventory {
    pub(crate) slots: [ItemStack; INVENTORY_SLOTS],
    pub(crate) selected_slot: u8,
    pub(crate) game_mode: GameType,
}

/// Encodes an inventory, failing on the first stack that is not strictly persistable.
pub(crate) fn encode(inventory: &DecodedInventory) -> Result<InventorySnapshot, String> {
    let mut entries = Vec::new();
    for (slot, stack) in inventory
        .slots
        .iter()
        .enumerate()
        .filter(|(_, stack)| !stack.is_empty())
    {
        stack
            .validate_persistent_encoding()
            .map_err(|error| format!("slot {slot}: {error}"))?;
        let NbtTag::Compound(mut entry) = stack.to_nbt_tag_ref() else {
            return Err(format!("slot {slot}: stack did not encode to a compound"));
        };
        let slot = i8::try_from(slot).map_err(|error| error.to_string())?;
        entry.insert(SLOT_KEY, NbtTag::Byte(slot));
        entries.push(entry);
    }

    let mut root = NbtCompound::new();
    root.insert(INVENTORY_KEY, NbtList::Compound(entries));
    root.insert(SELECTED_SLOT_KEY, i32::from(inventory.selected_slot));
    root.insert(GAME_TYPE_KEY, i32::from(inventory.game_mode));
    let mut bytes = Vec::new();
    NbtTag::Compound(root).write(&mut bytes);
    Ok(InventorySnapshot {
        format: INVENTORY_SNAPSHOT_FORMAT,
        data: bytes.into_iter().collect(),
    })
}

/// Decodes and validates a whole snapshot; nothing is applied on error.
pub(crate) fn decode(snapshot: &InventorySnapshot) -> Result<DecodedInventory, String> {
    if snapshot.format != INVENTORY_SNAPSHOT_FORMAT {
        return Err(format!(
            "unsupported inventory snapshot format {}, this host supports {INVENTORY_SNAPSHOT_FORMAT}",
            snapshot.format
        ));
    }
    let tag = read_tag(&mut Cursor::new(snapshot.data.as_slice()))
        .map_err(|error| format!("snapshot bytes are not NBT: {error}"))?;
    let tag = tag.as_tag();
    let root = tag.compound().ok_or("snapshot root is not a compound")?;

    let selected_slot = root
        .int(SELECTED_SLOT_KEY)
        .ok_or("snapshot has no SelectedItemSlot int")?;
    let selected_slot = u8::try_from(selected_slot)
        .ok()
        .filter(|slot| PlayerInventory::is_hotbar_slot(*slot))
        .ok_or_else(|| format!("SelectedItemSlot {selected_slot} is not a hotbar slot"))?;
    let game_type = root
        .int(GAME_TYPE_KEY)
        .ok_or("snapshot has no playerGameType int")?;
    let game_mode = GAME_TYPES
        .into_iter()
        .find(|mode| i32::from(*mode) == game_type)
        .ok_or_else(|| format!("unknown playerGameType {game_type}"))?;

    let list = root
        .list(INVENTORY_KEY)
        .ok_or("snapshot has no Inventory list")?;
    let mut slots: [ItemStack; INVENTORY_SLOTS] = array::from_fn(|_| ItemStack::empty());
    let mut seen = [false; INVENTORY_SLOTS];
    // An empty list has no element type, so it has no compound view.
    let entries = if list.empty() {
        None
    } else {
        Some(
            list.compounds()
                .ok_or("Inventory is not a list of compounds")?,
        )
    };
    for entry in entries.into_iter().flatten() {
        let slot = entry
            .byte(SLOT_KEY)
            .ok_or("Inventory entry has no Slot byte")?;
        let index = usize::try_from(slot)
            .ok()
            .filter(|index| *index < INVENTORY_SLOTS)
            .ok_or_else(|| format!("slot {slot} is outside 0..{INVENTORY_SLOTS}"))?;
        if mem::replace(&mut seen[index], true) {
            return Err(format!("slot {slot} appears more than once"));
        }
        slots[index] = ItemStack::from_borrowed_compound(&entry).ok_or_else(|| {
            format!("slot {slot}: unknown item, invalid components or count outside 1-99")
        })?;
    }
    Ok(DecodedInventory {
        slots,
        selected_slot,
        game_mode,
    })
}

pub(crate) fn inventory_snapshot(
    server: &CoreServer,
    id: PlayerId,
) -> Result<InventorySnapshot, String> {
    let player = find(server, id)?;
    // Stacks are cloned so the inventory lock is held only for the copy, not for encoding.
    let (slots, selected_slot) = {
        let inventory = player.inventory.lock();
        (
            array::from_fn(|slot| inventory.get_item(slot).clone()),
            inventory.get_selected_slot(),
        )
    };
    encode(&DecodedInventory {
        slots,
        selected_slot,
        game_mode: player.game_mode(),
    })
}

pub(crate) fn restore_inventory(
    server: &CoreServer,
    id: PlayerId,
    snapshot: &InventorySnapshot,
) -> Result<(), String> {
    let restored = decode(snapshot)?;
    let player = find(server, id)?;
    {
        let mut inventory = player.inventory.lock();
        for (slot, stack) in restored.slots.into_iter().enumerate() {
            inventory.set_item(slot, stack);
        }
        inventory.set_selected_slot(restored.selected_slot);
    }
    // A cancelled game mode change event keeps the old mode, as for `set_game_mode` elsewhere.
    player.set_game_mode(restored.game_mode);
    player.send_packet(CSetHeldSlot {
        slot: i32::from(restored.selected_slot),
    });
    player.broadcast_inventory_changes();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Debug;
    use steel_registry::{
        DyeColor, RegistryHolder, RegistryReference,
        data_components::{
            components::{CustomData, ItemEnchantments, ItemLore},
            vanilla_components::{
                self as components, ArmorTrim, BannerPatternLayer, BannerPatternLayers,
                BundleContents, CustomModelData, DyedItemColor, FireworkExplosion,
                FireworkExplosionShape, Fireworks, ItemAttributeModifierDisplay,
                ItemAttributeModifierEntry, ItemAttributeModifiers, ItemContainerContents,
                ItemStackTemplate, PotionContents,
            },
        },
        init_vanilla_registry, vanilla_attributes, vanilla_banner_patterns, vanilla_items,
        vanilla_potions, vanilla_trim_materials, vanilla_trim_patterns,
    };
    use steel_utils::Identifier;
    use text_components::TextComponent;

    fn ok<T, E: Debug>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("unexpected error: {error:?}"),
        }
    }

    fn err<T>(result: Result<T, String>) -> String {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(message) => message,
        }
    }

    fn empty_inventory() -> DecodedInventory {
        DecodedInventory {
            slots: array::from_fn(|_| ItemStack::empty()),
            selected_slot: 0,
            game_mode: GameType::Survival,
        }
    }

    /// One stack per persistent component of the acceptance set, spread over hotbar, main, armor
    /// and off hand.
    fn acceptance_inventory() -> DecodedInventory {
        let mut inventory = empty_inventory();
        inventory.selected_slot = 4;
        inventory.game_mode = GameType::Creative;
        add_visual_and_container_stacks(&mut inventory);

        let mut potion = ItemStack::new(&vanilla_items::POTION);
        potion.set(
            components::POTION_CONTENTS,
            PotionContents::new(
                Some(RegistryReference::new(&vanilla_potions::HEALING)),
                Some(0x0000_FF00),
                Vec::new(),
                Some("custom".to_owned()),
            ),
        );
        inventory.slots[0] = potion;

        let mut book = ItemStack::new(&vanilla_items::ENCHANTED_BOOK);
        let mut stored = ItemEnchantments::empty();
        stored.set(Identifier::vanilla_static("sharpness"), 3);
        book.set(components::STORED_ENCHANTMENTS, stored);
        inventory.slots[1] = book;

        let mut pickaxe = ItemStack::new(&vanilla_items::DIAMOND_PICKAXE);
        let mut enchantments = ItemEnchantments::empty();
        enchantments.set(Identifier::vanilla_static("efficiency"), 5);
        pickaxe.set(components::ENCHANTMENTS, enchantments);
        pickaxe.set(components::DAMAGE, 17);
        pickaxe.set(components::CUSTOM_NAME, TextComponent::plain("Prison Pick"));
        pickaxe.set(
            components::LORE,
            ok(ItemLore::new(vec![
                TextComponent::plain("one"),
                TextComponent::plain("two"),
            ])),
        );
        let mut custom = NbtCompound::new();
        custom.insert("owner", "someone");
        custom.insert("level", 7);
        let Some(custom) = CustomData::try_from_compound(custom) else {
            panic!("custom data should build");
        };
        pickaxe.set(components::CUSTOM_DATA, custom);
        pickaxe.set(
            components::CUSTOM_MODEL_DATA,
            CustomModelData::new(
                vec![1.5],
                vec![true, false],
                vec!["a".to_owned()],
                vec![0x0012_3456],
            ),
        );
        pickaxe.set(
            components::ATTRIBUTE_MODIFIERS,
            ItemAttributeModifiers {
                modifiers: vec![ItemAttributeModifierEntry {
                    attribute: vanilla_attributes::ATTACK_DAMAGE,
                    id: Identifier::new_static("test", "bonus"),
                    amount: 3.0,
                    operation: components::AttributeModifierOperation::AddValue,
                    slot: components::EquipmentSlotGroup::MainHand,
                    display: ItemAttributeModifierDisplay::Default,
                }],
            },
        );
        inventory.slots[2] = pickaxe;

        inventory
    }

    fn add_visual_and_container_stacks(inventory: &mut DecodedInventory) {
        let mut banner = ItemStack::new(&vanilla_items::WHITE_BANNER);
        banner.set(
            components::BANNER_PATTERNS,
            BannerPatternLayers::new(vec![BannerPatternLayer::new(
                RegistryHolder::reference(&vanilla_banner_patterns::CREEPER),
                DyeColor::Lime,
            )]),
        );
        inventory.slots[9] = banner;

        let explosion = FireworkExplosion::new(
            FireworkExplosionShape::Star,
            vec![0x00FF_0000],
            vec![0x0000_00FF],
            true,
            false,
        );
        let mut rocket = ItemStack::new(&vanilla_items::FIREWORK_ROCKET);
        rocket.set(
            components::FIREWORKS,
            ok(Fireworks::new(2, vec![explosion.clone()])),
        );
        inventory.slots[10] = rocket;
        let mut star = ItemStack::new(&vanilla_items::FIREWORK_STAR);
        star.set(components::FIREWORK_EXPLOSION, explosion);
        inventory.slots[11] = star;

        let mut shulker = ItemStack::new(&vanilla_items::SHULKER_BOX);
        shulker.set(
            components::CONTAINER,
            ok(ItemContainerContents::new(vec![
                Some(ItemStackTemplate::with_count(&vanilla_items::DIAMOND, 3)),
                None,
                Some(ItemStackTemplate::new(&vanilla_items::STICK)),
            ])),
        );
        inventory.slots[12] = shulker;

        let mut bundle = ItemStack::new(&vanilla_items::BUNDLE);
        bundle.set(
            components::BUNDLE_CONTENTS,
            BundleContents::new(vec![ItemStackTemplate::with_count(
                &vanilla_items::STONE,
                5,
            )]),
        );
        inventory.slots[13] = bundle;

        let mut chestplate = ItemStack::new(&vanilla_items::IRON_CHESTPLATE);
        chestplate.set(components::DYED_COLOR, DyedItemColor::new(0x0033_66CC));
        chestplate.set(
            components::TRIM,
            ArmorTrim::new(
                RegistryHolder::reference(&vanilla_trim_materials::IRON),
                RegistryHolder::reference(&vanilla_trim_patterns::SENTRY),
            ),
        );
        inventory.slots[PlayerInventory::SLOT_OFFHAND - 3] = chestplate;

        inventory.slots[PlayerInventory::SLOT_OFFHAND] =
            ItemStack::with_count(&vanilla_items::STONE, 64);
    }

    fn assert_same_inventory(actual: &DecodedInventory, expected: &DecodedInventory) {
        assert_eq!(actual.selected_slot, expected.selected_slot);
        assert_eq!(actual.game_mode, expected.game_mode);
        for (slot, (a, e)) in actual.slots.iter().zip(&expected.slots).enumerate() {
            assert_eq!(a.item(), e.item(), "item in slot {slot}");
            assert_eq!(a.count(), e.count(), "count in slot {slot}");
            assert!(a.components_equal(e), "components in slot {slot}");
        }
    }

    /// Builds a blob with raw entries, bypassing `encode`.
    fn blob(entries: Vec<NbtCompound>, selected: i32, game_type: i32) -> InventorySnapshot {
        let mut root = NbtCompound::new();
        root.insert(INVENTORY_KEY, NbtList::Compound(entries));
        root.insert(SELECTED_SLOT_KEY, selected);
        root.insert(GAME_TYPE_KEY, game_type);
        let mut bytes = Vec::new();
        NbtTag::Compound(root).write(&mut bytes);
        InventorySnapshot {
            format: INVENTORY_SNAPSHOT_FORMAT,
            data: bytes.into_iter().collect(),
        }
    }

    fn entry(slot: i8, id: &str, count: i32) -> NbtCompound {
        let mut entry = NbtCompound::new();
        entry.insert(SLOT_KEY, NbtTag::Byte(slot));
        entry.insert("id", id);
        entry.insert("count", count);
        entry
    }

    const SURVIVAL: i32 = GameType::Survival as i32;

    #[test]
    fn acceptance_components_round_trip_losslessly() {
        init_vanilla_registry();
        let inventory = acceptance_inventory();
        let decoded = ok(decode(&ok(encode(&inventory))));
        assert_same_inventory(&decoded, &inventory);
    }

    #[test]
    fn encoding_is_deterministic() {
        init_vanilla_registry();
        let inventory = acceptance_inventory();
        assert_eq!(ok(encode(&inventory)), ok(encode(&inventory)));
    }

    #[test]
    fn empty_inventory_round_trips() {
        init_vanilla_registry();
        let inventory = empty_inventory();
        let decoded = ok(decode(&ok(encode(&inventory))));
        assert_same_inventory(&decoded, &inventory);
        assert!(decoded.slots.iter().all(ItemStack::is_empty));
    }

    #[test]
    fn transient_components_are_dropped() {
        init_vanilla_registry();
        let mut inventory = empty_inventory();
        let mut stack = ItemStack::with_count(&vanilla_items::STONE, 2);
        stack.set(components::CREATIVE_SLOT_LOCK, ());
        inventory.slots[3] = stack;
        let decoded = ok(decode(&ok(encode(&inventory))));
        assert!(!decoded.slots[3].has(components::CREATIVE_SLOT_LOCK));
        assert_eq!(decoded.slots[3].count(), 2);
    }

    #[test]
    fn encode_names_the_slot_of_an_unencodable_stack() {
        init_vanilla_registry();
        let mut inventory = empty_inventory();
        inventory.slots[7] = ItemStack::with_count(&vanilla_items::STONE, 100);
        assert!(err(encode(&inventory)).contains("slot 7"));
    }

    #[test]
    fn decode_rejects_non_nbt_bytes() {
        let snapshot = InventorySnapshot {
            format: INVENTORY_SNAPSHOT_FORMAT,
            data: [0xFF_u8, 0x01, 0x02].into_iter().collect(),
        };
        assert!(err(decode(&snapshot)).contains("not NBT"));
    }

    #[test]
    fn decode_rejects_unknown_format() {
        let mut snapshot = blob(Vec::new(), 0, SURVIVAL);
        snapshot.format = INVENTORY_SNAPSHOT_FORMAT + 1;
        let message = err(decode(&snapshot));
        assert!(message.contains(&snapshot.format.to_string()));
        assert!(message.contains(&INVENTORY_SNAPSHOT_FORMAT.to_string()));
    }

    #[test]
    fn decode_rejects_bad_stacks_naming_the_slot() {
        init_vanilla_registry();
        for bad in [
            entry(5, "minecraft:not_an_item", 1),
            entry(5, "minecraft:stone", 0),
            entry(5, "minecraft:stone", 100),
        ] {
            let message = err(decode(&blob(vec![bad], 0, SURVIVAL)));
            assert!(message.contains("slot 5"), "{message}");
        }
    }

    #[test]
    fn decode_rejects_invalid_components() {
        init_vanilla_registry();
        let mut bad = entry(6, "minecraft:stone", 1);
        let mut components = NbtCompound::new();
        components.insert("minecraft:damage", "not a number");
        bad.insert("components", NbtTag::Compound(components));
        assert!(err(decode(&blob(vec![bad], 0, SURVIVAL))).contains("slot 6"));
    }

    #[test]
    fn decode_rejects_out_of_range_and_duplicate_slots() {
        init_vanilla_registry();
        for slot in [-1, i8::try_from(INVENTORY_SLOTS).unwrap_or(i8::MAX)] {
            let message = err(decode(&blob(
                vec![entry(slot, "minecraft:stone", 1)],
                0,
                SURVIVAL,
            )));
            assert!(message.contains(&format!("slot {slot}")), "{message}");
        }
        let duplicate = vec![
            entry(2, "minecraft:stone", 1),
            entry(2, "minecraft:dirt", 1),
        ];
        assert!(err(decode(&blob(duplicate, 0, SURVIVAL))).contains("slot 2"));
    }

    #[test]
    fn decode_rejects_bad_selected_slot_and_game_mode() {
        init_vanilla_registry();
        for selected in [-1, i32::from(PlayerInventory::SELECTION_SIZE)] {
            let message = err(decode(&blob(Vec::new(), selected, SURVIVAL)));
            assert!(message.contains(&selected.to_string()), "{message}");
        }
        let unknown_mode = i32::from(GameType::Spectator) + 1;
        let message = err(decode(&blob(Vec::new(), 0, unknown_mode)));
        assert!(message.contains(&unknown_mode.to_string()), "{message}");
    }
}
