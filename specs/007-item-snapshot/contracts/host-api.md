# Contract: host bindings (`steel-extension-api`)

New module `steel-extension-api/src/inventory_snapshot.rs` (types, plus a `#[stabby::host_api]` block of `Server<'_>` methods, same pattern as `items.rs`), adapter in `host/inventory_snapshot.rs`. Existing bindings keep their behavior and layout (FR-009).

Status: **proposed**. A published contract changes only with the user's approval.

## Types

```rust
/// Snapshot format understood by this host.
pub const INVENTORY_SNAPSHOT_FORMAT: u32 = 1;

#[stabby::stabby]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventorySnapshot {
    /// `INVENTORY_SNAPSHOT_FORMAT` when produced by this host.
    pub format: u32,
    /// Binary NBT, see "Blob" below. Opaque to extensions; persist it as bytes.
    pub data: ApiVec<u8>,
}
```

Only ABI-stable fields (`u32`, `stabby::vec::Vec<u8>`). Changing `data`'s meaning means a new `format`; changing the struct layout means a new type, both with the user's approval.

## Methods on `Server<'_>`

| Method | Result | Behavior |
| --- | --- | --- |
| `inventory_snapshot(player: PlayerId)` | `ApiResult<InventorySnapshot>` | Reads slots 0-40, selected hotbar slot, game mode. Fails naming the slot if a stack cannot be encoded strictly (D4). Does not modify the player. |
| `restore_inventory(player: PlayerId, snapshot: &InventorySnapshot)` | `ApiResult<()>` | Decodes and validates everything first, then applies the game mode (`Player::set_game_mode`; an unchanged mode is fine, a change cancelled by an event is an error), then sets slots 0-40 (absent slots empty) and the selected slot, then resyncs the client and sends the held slot. On error the player is unchanged. |

Both return `Err` for an unknown or offline player and never block. Neither calls extension code while holding a host lock.

## Blob

Binary NBT, one root compound in the nameless-root (network) form, written with `simdnbt::owned::NbtTag::write` and read with `simdnbt::borrow::read_tag` (confirmed in T001):

```text
{
  Inventory: [ { Slot: byte, id: string, count: int, components?: compound }, ... ],  // non-empty slots only, ascending Slot, Slot in 0..=40
  SelectedItemSlot: int,                                                              // 0..=8
  playerGameType: int                                                                 // 0 survival, 1 creative, 2 adventure, 3 spectator
}
```

- Each list entry is the vanilla `ItemStackWithSlot` shape; `id`, `count`, `components` are produced by Steel's persistent `ItemStack` codec (`ItemStack::to_nbt_tag_ref` after a strict `try_to_nbt_tag_ref` check).
- Slots 36-40 are armor (feet to head) and off hand in the **extension API's flat numbering**. Vanilla 26.x stores those under `equipment`; this blob does not claim `/data` compatibility for them.
- Slots 41-42 are not present (O1).

## Errors (messages are stable text for logs, not for matching)

| Cause | Message names |
| --- | --- |
| Unknown player | player id |
| Stack not strictly encodable | slot, and the component error |
| Unknown `format` | the number received and the number supported |
| Bytes not NBT or not the expected shape | what was missing |
| Unknown item id, invalid components, count outside 1-99 | slot |
| `Slot` outside 0-40, duplicate `Slot` | slot |
| `SelectedItemSlot` outside 0-8, unknown `playerGameType` | the value |
| Game mode change cancelled by an event | the requested mode |
