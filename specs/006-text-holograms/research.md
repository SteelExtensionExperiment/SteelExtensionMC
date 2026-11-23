# Research: Extension-controlled Text Holograms

File:line references are relative to the Steel checkout (`SteelExtensionMC/`) and were checked on
2026-11-23 (`master` at `55a3fb29`). Re-verify before implementing.

## R1. Vanilla reference (resolved 2026-11-23)

**Source.** `/home/jonas/minecraft/minecraft_26.3/net/minecraft/` (`version.json` id `26.3`, matches `Cargo.toml`
`0.17.1+mc26.3`). The repo's `minecraft-src/` is not generated; this local copy was used instead.

**Verified against `world/entity/Display.java` and `steel-registry/build_assets/entities.json`.** A1 holds:

- Defaults: line width 200, text opacity -1, background 1073741824 (`0x4000_0000`), view range 1.0, billboard
  `FIXED`; same constants as `text_display.rs:24-33` and `display/mod.rs:28-41`.
- Billboard ids: `FIXED 0, VERTICAL 1, HORIZONTAL 2, CENTER 3`; Steel's `BillboardConstraints` matches (`display/mod.rs:380-390`).
- Entity type `text_display`: `clientTrackingRange(10)`, `updateInterval(1)`, `can_serialize: true`.
- Client render distance is `view_range * 64 * viewScale` blocks (`shouldRenderAtSqrDistance`), so default 64.
- Text travels as `ComponentSerialization.TRUSTED_STREAM_CODEC` (entity data). Vanilla has no server-side line
  or length limit; `line_width` only wraps on the client. Closes the text-limit part of O1: no host-side limit
  beyond what the protocol and `text.rs` parsing already enforce.
- Vanilla persists a text display through `addAdditionalSaveData` because the type is serializable; non-persistence
  is therefore a Steel-side override (R4), not vanilla behavior.

## R2. The existing text display entity

**Finding.**

- `TextDisplayEntity` exists, with `entity_data: SyncMutex<TextDisplayEntityData>`, created by the
  `ENTITIES` factory and spawnable (`steel-core/src/entity/entities/objects/technical/display/text_display.rs:38-75`).
- Text is changed through a view: `TextDisplayView::set_text` and `text` (`text_display.rs:177-210`), reached
  through `Display::with_view` (`display/mod.rs:95`).
- Billboard mode: `BillboardConstraints` and `set_billboard_constraints`
  (`display/mod.rs:273-280,380-`).
- Defaults: line width 200, opacity -1, background `0x4000_0000` (`text_display.rs:24-33`).

**Decision.** Reuse `TextDisplayEntity`; do not add a new entity type.

## R3. Existing extension spawn path and why it is not enough

**Finding.** `World::spawn_entity(type, pos)` returns only a UUID, requires a loaded chunk and spawns with
defaults (`steel-extension-api/src/entity_behavior.rs:68-75`, host `steel-extension-api/src/host/entity.rs:64-84`).
`World::remove_entity` takes an entity id (`steel-extension-api/src/worlds.rs:107`). There is no way to
set text. An entity spawned this way is persistent (`Entity::is_persistent` returns the entity type's
`can_serialize`, `steel-core/src/entity/entity/mod.rs:172-176`), so it would be saved and duplicated on
restart, which is why H2 is needed.

**Decision.** New hologram bindings instead of a general "edit entity data" binding, because the stats board
only needs text and the lifecycle (H2 to H4) is the hard part.

## R4. Non-persistence and chunk unload precedent

**Finding.**

- `Entity::is_persistent` is the existing override point ("Steel's temporary block seats override the
  vanilla entity-type default", `entity/entity/mod.rs:172-176`; `nbt_for_passenger_save` returns nothing
  when it is false, `:348`). `BlockSeat` is a Steel-owned, never-persisted entity with a registry
  (`steel-core/src/entity/block_seat.rs:1-45`) and is the closest precedent.
- Chunk unload hooks: `World::on_entity_chunk_unload_start` and `..._finalized`
  (`steel-core/src/world/entity_management.rs:377-400`), backed by `entity_manager.begin_chunk_unload`.
- World load and unload events already reach extensions
  (`steel-extension-api/src/events/handlers.rs:378-388`).

**Resolved (2026-11-23).** With `is_persistent() == false` the entity manager does not save the entity
(`entity/manager/mod.rs:511-518` `should_save`, `chunk_saver/storage/entities.rs:226-235`), and `BlockSeat`
does exactly this (`block_seat.rs:85`) and is dropped with its chunk. Chunk hooks that fire per chunk:
`World::on_entity_chunk_loaded` (`entity_management.rs:340`, after `entity_manager.on_chunk_loaded`) and
`on_entity_chunk_unload_finalized` (`:393`). H3 mechanism: **remove and recreate**; the registry keeps the
record (text, position, billboard), drops the entity on unload and spawns a fresh one on load.

**Decision.** A non-persistent text display needs either a flag on `TextDisplayEntity` or a thin Steel-owned
wrapper that overrides `is_persistent`. It must not change `/summon text_display` (FR-008).

## R5. Risks and open points

1. ~~Vanilla unverified~~: resolved, R1.
2. **Chunk lifecycle** (R4): the H3 mechanism is not yet designed against the real unload hooks.
3. **Handle ownership** (H6): dropped by the user. `Server<'a>` wraps only `&Arc<CoreServer>` (`runtime/bindings.rs:27`), no caller identity; `Operation` handles are unowned too.
4. ~~Text limits~~: no vanilla server limit, R1.
