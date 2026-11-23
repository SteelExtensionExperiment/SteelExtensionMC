# Implementation Plan: Lossless Inventory Snapshot (host platform)

**Branch**: `host/007-item-snapshot` | **Date**: 2026-11-23 | **Spec**: [spec.md](spec.md)

## Summary

Add `InventorySnapshot` and two `Server` bindings (`inventory_snapshot`, `restore_inventory`) to `steel-extension-api`. The host side reuses Steel's persistent `ItemStack` codec for each stack and wraps them in a vanilla-named binary NBT blob. No new component code, no new dependency, no registry or extractor data.

## Technical context

- Crates touched: `steel-extension-api` only (`src/inventory_snapshot.rs`, `src/host/inventory_snapshot.rs`, `lib.rs` and `host/mod.rs` registration). `steel-core` and `steel-registry` are used, not changed, unless T003 finds the strict encode path or an NBT byte writer is missing (then: stop and ask).
- Build: `export CARGO_BUILD_JOBS=3`. Check with `cargo check -p steel-extension-api --features host`, tests with `cargo test -p steel-extension-api --features host inventory_snapshot` (exact feature flags to confirm against the existing `items.rs` tests when T001 runs).
- Vanilla reference: `/home/jonas/minecraft/minecraft_26.3/net/minecraft/` (research R3).

## AGENTS.md check

| Rule | How this plan meets it |
| --- | --- |
| Vanilla functionality | Item and component encoding is Steel's existing vanilla codec; non-persistent components follow vanilla (R4). The blob shape is documented as ours where it differs (R3). |
| No hardcoded vanilla data | None added. Slot count and game mode ids are named constants tied to the vanilla `Inventory` constants and `GameType` legacy ids; no item or component list in code. |
| Generated code | Nothing generated is edited. |
| No magic numbers | `INVENTORY_SNAPSHOT_FORMAT`, `SNAPSHOT_SLOTS` (41), `HOTBAR_SIZE` reuse the existing `INVENTORY_SLOTS` in `host/items.rs` where possible (T002). |
| Sketchy workarounds | No `.clone()` to appease the borrow checker (stacks are cloned out of the lock deliberately to hold it briefly; if reviewers consider that a workaround, T004 notes it), no `unwrap`/`expect`, no `unsafe`, no downcasting. |
| Game ticks never wait | Both bindings are synchronous and short. |
| Tests | Behavior-focused: round trip of the real component kinds, atomic failure, determinism. No constant-restating tests. |

## Design

**Capture** (`host/inventory_snapshot.rs`): lock the player's inventory, clone slots 0-40 into a local array, read selected slot and game mode, release the lock. Then, lock-free, for each non-empty stack: strict check, `to_nbt_tag_ref`, insert `Slot`; build the root compound; write bytes.

**Restore**: parse bytes, check `format`, decode each entry with `ItemStack::from_borrowed_compound`, validate slots and values into a local `DecodedInventory { slots: [ItemStack; 41], selected: u8, mode: GameMode }`. Only after that: lock the inventory, `set_item` for 0..41, `set_selected_slot`, unlock, `Player::set_game_mode`, `broadcast_inventory_changes` (as `set_slot` does).

**Pure core**: `encode(&[ItemStack], selected, mode) -> Result<InventorySnapshot, String>` and `decode(&InventorySnapshot) -> Result<DecodedInventory, String>` take no server, so unit tests run against the real registry (`init_vanilla_registry`) like `host/items.rs` tests do.

**Why not per-component API types**: every new vanilla component would need a new stabby type and conversion; the persistent codec already covers them and is what survives restarts.

**Why not one opaque `ItemStack` network blob**: the network codec is lossy by design (`client_content::client_stack` rewrites stacks for the client) and is not the persistent form.

## Platform gaps to resolve before or during T003 (stop and ask if confirmed missing)

1. ~~Binary NBT writer~~ resolved in T001: `NbtTag::Compound(c).write(&mut Vec<u8>)` and `simdnbt::borrow::read_tag` (see `steel-utils/src/nbt/value_input.rs:250`).
2. ~~Strict encode access~~ resolved in T001: `ItemStack::validate_persistent_encoding` and `patch().try_to_nbt_tag_ref()` are `pub`.
3. Data version stamp (O2) and slots 41-42 (O1): user decisions, not code gaps.

## Project structure

```text
specs/007-item-snapshot/{spec,plan,tasks,research}.md, contracts/host-api.md
steel-extension-api/src/inventory_snapshot.rs        # types + host_api block
steel-extension-api/src/host/inventory_snapshot.rs   # encode/decode + adapters + tests
```

Split `encode`/`decode` into a second file if the adapter file passes about 400 lines.

## Milestone for dependents

After T010: `InventorySnapshot`, `Server::inventory_snapshot`, `Server::restore_inventory` are usable. `arena-manager-api` can add `PlayerEntry.kit: InventorySnapshot` (type exported from `steel_extension_api`), and the orchestrator can persist `data`.
