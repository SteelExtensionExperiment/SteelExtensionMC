# Research: Lossless Inventory Snapshot

Verified 2026-11-23 in this checkout (`host/007-item-snapshot`) and in vanilla `26.3` at `/home/jonas/minecraft/minecraft_26.3/net/minecraft/`. `minecraft-src/` is not generated here, so vanilla paths below are relative to that directory.

## R1. The current API is lossy by construction

`ItemSpec` holds item, name, lore, enchantments, extension data, count, glint, unbreakable (`steel-extension-api/src/items.rs:55-70`); `host/items.rs::to_stack` builds a fresh stack from those fields and `to_spec` reads only them. Everything else (potion contents, stored enchantments, custom model data, attribute modifiers, banner patterns, fireworks, container contents) is lost on a read-then-write. Not fixable by adding fields: each new component needs a new API type.

## R2. Steel already has a lossless persistent item codec

- `ItemStack::to_nbt_tag_ref` writes `{ id, count, components? }`; `ItemStack::from_nbt_tag` and `from_borrowed_compound` read it (`steel-registry/src/item_stack.rs:954-1066`). Count range on read is 1..=99 (`decode_persistent_count`, `item_stack.rs:1070`).
- The component patch is encoded by `DataComponentPatch::to_nbt_tag_ref` (save-time: invalid fields are reported and **omitted**) and `try_to_nbt_tag_ref` (strict: error) in `steel-registry/src/data_components/registry/patch_persistence.rs:104-122`. `ItemStack::validate_persistent_encoding` uses the strict one (`item_stack.rs:976`).
- Player save uses it: `PersistentPlayerData` collects slots `0..PlayerInventory::CONTAINER_SIZE` (43) as `PersistentSlot { slot, item }` (`steel-core/src/player/player_data.rs:217-226`) and restores them at `:484-500`. So the codec already survives a restart for every component Steel supports.
- `PlayerInventory::to_vanilla_inventory_nbt` produces the vanilla `ItemStackWithSlot` list for slots 0..36 only (`steel-core/src/player/player_inventory/core.rs:111-127`); armor and off hand are not in it.

**Consequence:** the snapshot reuses this codec (D1). It adds no component code. The strict path is used for capture (D4) because the save-time path would silently drop a field, which is exactly the loss this spec exists to prevent.

## R3. Vanilla shape

- `Inventory.save` writes only non-empty entries of the 36 `items` as `ItemStackWithSlot(i, item)`; `load` clears and sets each valid slot (`world/entity/player/Inventory.java:393-410`).
- `ItemStackWithSlot.CODEC` = `Slot` (unsigned byte, default 0) + `ItemStack.MAP_CODEC` fields inline (`world/ItemStackWithSlot.java`).
- `Player.addAdditionalSaveData` writes `Inventory` and `SelectedItemSlot` (`world/entity/player/Player.java:642-643`); `ServerPlayer` writes `playerGameType` with `GameType.LEGACY_ID_CODEC` (`server/level/ServerPlayer.java:2093`).
- Slot constants: `INVENTORY_SIZE` 36, armor 36-39 (feet, legs, chest, head), `SLOT_OFFHAND` 40, `SLOT_BODY_ARMOR` 41, `SLOT_SADDLE` 42 (`Inventory.java:33-47`). In vanilla 26.x armor and off hand are held by `EntityEquipment`, saved under the entity's `equipment` field, not in `Inventory`; Steel's player save uses its own flat 43-slot list. The snapshot uses the flat slot numbers (0-40) of the extension API (`items.rs:13-14`), written in the `Inventory` list shape. This is **our** blob shape inspired by vanilla names, not a vanilla file; it is stated in the contract so nobody expects `/data` compatibility for slots 36-40.

## R4. Components vanilla does not persist

In `core/component/DataComponents.java`, a type without `.persistent(...)` is network only. Found: `creative_slot_lock` (`:170`), `additional_trade_cost` (`:228`). Vanilla drops them when it saves a stack, so a snapshot dropping them is vanilla behavior (D8). Steel's `is_persistent()` on the registry entry is what implements this; the plan task T010 lists which entries are non-persistent in Steel and compares the list with vanilla's.

## R5. Existing inventory and game mode bindings

- `Server::slot/set_slot/give/set_item_data`, slot range check `0..41` (`host/items.rs:166-176`), lock then `player.broadcast_inventory_changes()` after writes (`:178-200`).
- `Server::game_mode` (`players.rs:202`), `Server::set_game_mode` (`runtime/bindings.rs:171`, adapter `host/runtime.rs:177-180` calling `Player::set_game_mode`, `steel-core/src/player/game_mode/player_methods.rs:22`). `GameMode` has Survival, Creative, Adventure, Spectator (`runtime/types.rs:70-78`); conversion helpers in `host/events.rs:727`.
- `clear_inventory` clears the container with `clear_content` (`host/runtime.rs:193`); its effect on slots 41-42 is not checked here.
- `simdnbt` is already an optional dependency of `steel-extension-api` under the `host` feature (`Cargo.toml:10`), so no new dependency.

## R6. What is missing or unknown

- No data-version constant in Steel (searched `DataVersion`, `world_version`, `5023` in `steel-core`, `steel-utils`, `steel-registry`, `steel-extension-api`): O2.
- Binary NBT writing helper: Steel stores player data through its own storage; the exact simdnbt owned-tag writer call to produce a named-root byte vector must be confirmed in T003 (read `steel-core/src/player/player_data_storage/file_storage.rs`). Not guessed here.
- Whether `PlayerInventory::set_item` for slots 36-40 triggers equipment-change handling the same way as `set_slot` does today: `set_slot` already uses it, so restore uses the same call (`container.rs:201-222`).
- Behavior of restore during a pending world or domain switch: O3.

## R7. Downstream use (no change needed here)

- `arena-manager-api` adds `PlayerEntry.kit: InventorySnapshot` (field of a stabby struct; the type comes from `steel-extension-api`, so arena-manager-api needs no other dependency).
- The orchestrator stores `InventorySnapshot.data` with its kit configuration and calls `restore_inventory` when a match starts. Hook actions are emitted after locks are released (`specs/extension-communication.md`); this spec's bindings take the inventory lock only briefly and call no extension code (FR-010).
