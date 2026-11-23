# Feature Specification: Lossless Inventory Snapshot (host platform)

**Feature Branch**: `007-item-snapshot`

**Created**: 2026-11-23

**Status**: Draft, **awaiting user approval** (nothing is implemented)

**Input**: Game orchestrator spec O1 and research R1 (`extension-example/game-orchestrator/specs/001-game-orchestrator/`): the extension API's `ItemSpec` drops every component it does not model, so a kit built from an administrator's inventory would not equal what `/invsee` shows. Needed: a host capability that reads and writes a player's full inventory (slots 0-40, game mode) as a lossless, ABI-stable value, including potions, enchanted books, custom model data, attribute modifiers, banners and fireworks. Downstream: `arena-manager-api` adds `PlayerEntry.kit`, the orchestrator saves and loads kits.

Vanilla target: `0.17.1+mc26.3`. The repo's `minecraft-src/` is not generated, so vanilla was read from `/home/jonas/minecraft/minecraft_26.3/net/minecraft/` (`version.json` id `26.3`). See [research.md](research.md).

## Decisions proposed (all need the user's confirmation)

| # | Proposal |
| --- | --- |
| D1 | **The stack format is Steel's existing persistent `ItemStack` codec** (vanilla `ItemStack.CODEC`: `id`, `count`, `components` patch). No second item serializer, no per-component code. A component the persistent codec can round-trip is therefore round-tripped; one it cannot is rejected (D4), never dropped. |
| D2 | **The value is one stabby struct holding a format number and a byte blob.** The blob is the vanilla player-save shape in binary NBT: `Inventory` (list of `ItemStackWithSlot`: `Slot`, `id`, `count`, `components`), `SelectedItemSlot`, `playerGameType`. Extensions treat it as opaque and persist `data` as they like. |
| D3 | **Slots 0-40 only**, as the brief says and as `Server::slot` does: 0-8 hotbar, 9-35 main, 36-39 armor (feet to head), 40 off hand. Vanilla's player save also covers 41 (body armor) and 42 (saddle) and Steel's `PlayerInventory` has 43 slots; they are **left out** (see O1). |
| D4 | **Capture is strict.** It encodes each stack through the strict path (`DataComponentPatch::try_to_nbt_tag_ref`, as `validate_persistent_encoding` does) and fails naming the slot if a stack cannot be encoded, instead of using the save-time path that logs and omits invalid fields. A snapshot is therefore lossless or absent. |
| D5 | **Restore is all-or-nothing.** The blob is fully decoded and validated (known item ids, valid components, counts 1-99, slot range, game mode id) before the player is touched. On any error the player is unchanged. |
| D6 | **Restore replaces slots 0-40**, so slots absent from the snapshot become empty. It sets the selected hotbar slot and the game mode. The game mode goes through the same path as `Server::set_game_mode`, so abilities and client packets follow vanilla. Slots 41 and 42, XP, health, effects and the ender chest are not touched. |
| D7 | **No migrations.** A snapshot carries a `format` number only. Restore rejects a format it does not know. The project is in early development (AGENTS.md), and Steel has no data-version constant to stamp (research R6); see O2. |
| D8 | **Non-persistent components are not captured**, exactly as vanilla does not save them: those declared with `networkSynchronized` only, such as `creative_slot_lock` and `additional_trade_cost` (research R4). A kit is a saved inventory, not a live copy. |

## User Scenarios & Testing *(mandatory)*

The "user" is an extension author calling the bindings in [contracts/host-api.md](contracts/host-api.md).

### User Story 1 - Snapshot an inventory and restore it unchanged (Priority: P1)

An administrator builds a kit in their own inventory (a splash potion, an enchanted book, a banner with patterns, a firework rocket, a sword with attribute modifiers and custom model data, armor, a shulker box with contents). An extension captures it and later restores it onto another player.

**Why this priority**: it is the whole purpose; kits in the game orchestrator depend on it.

**Independent Test**: fill slots 0-40 with stacks holding the components above, capture, clear, restore, then compare every slot by item, count and full component equality.

**Acceptance Scenarios**:

1. **Given** a populated inventory, **When** captured and restored on the same player after clearing, **Then** every slot matches by item, count and every persistent component (`ItemStack::components_equal`).
2. **Given** a snapshot captured from player A, **When** restored on player B in another game mode, **Then** B's slots, selected slot and game mode equal A's at capture time.
3. **Given** a snapshot, **When** restored twice, **Then** the second restore changes nothing visible.

### User Story 2 - Persist the snapshot across restarts (Priority: P1)

The orchestrator writes `data` to its own storage and reads it back after a server restart.

**Independent Test**: capture, write the bytes to a file, read them back into a new snapshot, restore, compare as in story 1.

**Acceptance Scenarios**:

1. **Given** bytes written by one run, **When** read back in a later run and restored, **Then** the result equals story 1.
2. **Given** the same inventory captured twice, **When** the bytes are compared, **Then** they are identical (deterministic: slots in ascending order).

### User Story 3 - A bad snapshot never half-applies (Priority: P2)

**Acceptance Scenarios**:

1. **Given** bytes that are not NBT, or hold an unknown item id, an invalid component, or a count outside 1-99, **When** restored, **Then** the call returns an error naming the cause and the player's inventory, selected slot and game mode are unchanged.
2. **Given** a `format` the host does not know, **When** restored, **Then** the call fails with the format in the message.

### Edge Cases

- Empty inventory: valid snapshot with an empty `Inventory` list; restore empties slots 0-40.
- A stack above 99 (a plugin-made oversized stack): capture fails with the slot (D4); it does not clamp.
- A player with an inventory screen or cursor item open: only slots 0-40 of the player inventory are read or written; the cursor item is not part of the snapshot.
- Offline or unknown player: error, as the existing slot bindings.
- Restore while the player is in the middle of a world or domain switch: not specified here; it behaves as `set_slot` does today (O3).
- After restore, the client sees the new inventory the same tick (the same resync `set_slot` triggers).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `Server::inventory_snapshot(player)` MUST return an `InventorySnapshot` of slots 0-40, the selected hotbar slot and the game mode.
- **FR-002**: Each non-empty stack MUST be encoded with the vanilla persistent `ItemStack` codec as implemented by Steel, using the strict path (D1, D4).
- **FR-003**: Capture MUST fail with a message naming the slot when a stack cannot be encoded; it MUST NOT omit components.
- **FR-004**: `Server::restore_inventory(player, &snapshot)` MUST decode and validate the whole snapshot before changing the player (D5).
- **FR-005**: Restore MUST set slots 0-40 (absent slots empty), the selected hotbar slot, and the game mode as `Server::set_game_mode` does (D6). It MUST NOT touch slots 41-42, XP, health, effects, ender chest or cursor.
- **FR-006**: The snapshot bytes MUST be vanilla-shaped binary NBT (D2) with slots in ascending order, so equal inventories give equal bytes.
- **FR-007**: `InventorySnapshot` MUST be `#[stabby::stabby]` with only ABI-stable fields, like `ItemSpec`; layout changes need a new `format` or key and the user's approval (workflow: published contracts).
- **FR-008**: Restore MUST reject an unknown `format`.
- **FR-009**: The existing `ItemSpec`, `slot`, `set_slot`, `give`, `set_item_data` bindings MUST keep their behavior and layout.
- **FR-010**: Capture and restore MUST NOT call extension code or emit hook actions while holding the inventory lock (`specs/extension-communication.md`); they hold the lock only to copy or write stacks.
- **FR-011**: Host tests MUST cover the round trip of the component kinds named in the input, using the real registry, and a failing restore leaving the player unchanged.
- **FR-012**: The game mode id in the blob MUST use vanilla's legacy numeric id (survival 0, creative 1, adventure 2, spectator 3), as `playerGameType`.

### Key Entities

- **InventorySnapshot**: `{ format: u32, data: ApiVec<u8> }`.
- **Snapshot blob**: root compound `{ Inventory: [ { Slot: byte, id: string, count: int, components?: compound } ], SelectedItemSlot: int, playerGameType: int }`.

## Success Criteria *(mandatory)*

- **SC-001**: A kit containing every component kind in the input round-trips with zero differences in the host test (FR-011).
- **SC-002**: Restore of a corrupt snapshot leaves 100 % of the player's observable state unchanged (tested for each error kind in story 3).
- **SC-003**: Capture plus restore of a full inventory completes inside one tick's slack. Measured in T012; the number is a proposal to confirm (expected well under 1 ms of host work).

## Assumptions

- **A1**: The snapshot is host-defined and lives in `steel-extension-api` (like `ItemSpec`), so `arena-manager-api` links `steel-extension-api` only and no extension links another's types for it.
- **A2**: Steel's persistent item codec already matches vanilla (it is the codec Steel saves player data with; `item_stack.rs:954-1022`). This spec does not re-verify each component's codec; spot checks are in T010.
- **A3**: Out of scope: snapshots of other containers (ender chest, chests), XP, health, effects, per-item snapshots (see O4), a diff or merge of snapshots, kit editing inside the blob.

## Open items for the user

- **O1 (confirm)**: slots 41 (body armor) and 42 (saddle) exist in vanilla's `Inventory` and in Steel's 43-slot `PlayerInventory`. The brief says 0-40. Including them is a one-constant change. Proposed: stay with 0-40 (D3).
- **O2 (confirm)**: stamp the blob with a data version? Steel has no constant for the target world version (vanilla `version.json` has `world_version` 5023). Adding one needs an agreed source (extractor or `Cargo.toml` suffix). Proposed: no stamp now (D7).
- **O3 (confirm)**: should restore also be allowed to skip the game mode (`restore_inventory(player, &snapshot, apply_game_mode)`), for kits that must not change mode? Proposed: no flag now; the orchestrator can set the mode back, and a later `v2` can add one.
- **O4 (confirm)**: also expose a single-stack snapshot (`slot_snapshot` / `set_slot_snapshot`) so `ItemSpec`-based commands can stop losing components? Not needed by the orchestrator. Proposed: no.
- **O5 (confirm)**: which exact component list is the acceptance set for SC-001? Proposed: potion contents, stored enchantments (book), enchantments, custom model data, attribute modifiers, banner patterns, fireworks and firework explosion, container contents (shulker box), bundle contents, custom name and lore, custom data, damage, dyed color, trim.
