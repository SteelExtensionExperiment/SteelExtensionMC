---
description: "Task list for the lossless inventory snapshot (host platform)"
---

# Tasks: Lossless Inventory Snapshot (host platform)

**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md), [contracts/host-api.md](contracts/host-api.md)

**Gate**: no task starts before the user approves the spec and answers O1-O5. Host changes are serialized by the user (`specs/extensions-workflow.md`).

**Tests**: required (FR-011). Unit tests sit beside the code in `host/inventory_snapshot.rs`; the native test is T011.

**Format**: `[ID] [P?] Description`. `[P]` = different files, no dependency on an incomplete task.

## Phase 1: Foundations

- [x] T001 Confirm the check and test commands and feature flags from the existing `host/items.rs` tests; read `steel-core/src/player/player_data_storage/file_storage.rs` for the simdnbt named-root byte writer and `host/events.rs:727` for the `GameMode` conversions. Record the exact calls at the top of `host/inventory_snapshot.rs`. **If the byte writer or the strict encode path is not reachable, stop and ask (plan, platform gaps).**
- [x] T002 Add `steel-extension-api/src/inventory_snapshot.rs`: `INVENTORY_SNAPSHOT_FORMAT`, `InventorySnapshot` (stabby), the `#[stabby::host_api]` block with `inventory_snapshot` and `restore_inventory` (adapter calls only), module registration in `lib.rs`; re-export next to `ItemSpec`. Reuse the slot-count constant from `host/items.rs` instead of a second literal.

## Phase 2: Pure encode and decode

- [x] T003 Add `host/inventory_snapshot.rs` with `encode` (strict check per stack, `Slot` insertion, ascending order, root compound, bytes) and `DecodedInventory` + `decode` (format check, shape check, `from_borrowed_compound`, slot range, duplicate `Slot`, selected slot 0-8, game mode id, count 1-99). All failures are `Err(String)` per the contract table. No `unwrap`/`expect`.
- [x] T004 Tests for T003 (real registry): (a) round trip of the O5 acceptance set, compared with `ItemStack::components_equal` slot by slot; (b) two encodes of the same inventory give equal bytes; (c) empty inventory; (d) one test per error kind in the contract table returns `Err` and a non-NBT byte string returns `Err`; (e) a stack with count above 99 fails encode naming its slot; (f) a network-only component (`creative_slot_lock`) is absent after the round trip (D8).

## Phase 3: Host adapters

- [x] T005 `inventory_snapshot(server, id)`: find player, lock inventory, copy slots 0-40 and selected slot, unlock, read game mode, `encode`. No extension code runs under the lock.
- [x] T006 `restore_inventory(server, id, &snapshot)`: `decode` first, then lock, set slots 0-40, selected slot, unlock, `Player::set_game_mode`, `broadcast_inventory_changes`. Slots 41-42 untouched.
- [x] T007 Register the adapters in `host/mod.rs`, run `cargo check` for the crate with the host feature and the narrowest test filter (`inventory_snapshot`).

## Phase 4: Verification

- [x] T010 Spot-check against vanilla for the O5 components: for each, confirm in `world/item/...` or `core/component/DataComponents.java` that the component is `persistent`, and list Steel registry entries where `is_persistent()` is false; compare with vanilla's network-only list (R4). Report any mismatch to the user; do not patch component codecs here.
- [ ] T011 Native probe test, modeled on `tests/event-probe` and spec 005 T001: a probe extension with one command that captures, clears, restores and compares through the public binding. `#[ignore]` until the probe is built. Skippable if the user accepts unit tests only (plan decision).
- [ ] T012 Measure capture plus restore of a full inventory in a unit test with `Instant` printed (not asserted) to confirm SC-003, then report the number to the user.
- [x] T013 Document in `steel-extension-api` module docs: lossless semantics, strict failure, slots 0-40, game mode applied, non-persistent components (D8), and that `ItemSpec` stays lossy by design.

## Phase 5: Hand-off

- [ ] T014 Report the milestone to the user: types and bindings landed; `arena-manager-api` can add `PlayerEntry.kit: InventorySnapshot`. Update the status row for 007 in `specs/extensions-workflow.md` only if the user asks (this stream does not edit shared files).

## Dependencies

T001 -> T002, T003 -> T004 -> T005, T006 -> T007 -> T010..T013 (T010, T012, T013 are `[P]` after T007) -> T014.

## Status 2026-11-24

T001-T007, T010, T013 done (commit `8ad86a34`). Open: T011 native probe (optional), T012 timing, T014 hand-off. Known deviations: restore also sends `CSetHeldSlot`; a cancelled game mode change event keeps the old mode and restore still succeeds (user decision pending).
