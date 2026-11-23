# Research: Runtime RAM Worlds from Saved Templates (host platform)

All file:line references were checked against the working tree on 2026-11-23 (`master` at
`55a3fb29`). Re-verify before implementing; the host moves quickly.

## R1. What exists today, and the gap

**Finding.** The extension API can create worlds only from a registered generator and only as
temporary worlds.

- `Server::create_temporary_world(key, generator, settings, primary)` is at
  `steel-extension-api/src/runtime/bindings.rs:99-115`. The adapter at
  `steel-extension-api/src/host/runtime.rs:98-117` always passes an empty generator configuration
  (`runtime.rs:110`).
- The core entry point is `Server::create_temporary_world` in
  `steel-core/src/server/temporary/mod.rs:86-176`. Preparation is
  `server/temporary/preparation.rs:39-123`: always `WorldStorageConfig::RamOnly` and
  `level_data_path: None` (`preparation.rs:60-85`), with a radius-1 pregeneration around spawn
  (`preparation.rs:110-121`).
- The extension `Terrain` handle exposes only chunk bounds, `random_at` and `write_blocks`
  (`steel-extension-api/src/bindings.rs:268-310`). It has no world identity and no source world, so
  a generator cannot know which template to copy (`host/generator.rs:140-150`).
- Reads are one block at a time and only on loaded worlds (`bindings.rs:211-215`,
  `host/mod.rs:527-538`). There is no chunk-level read and no file access
  (`steel-extension-api/src/fill.rs:224-258` writes weighted patterns, never per-position arrays).
- Extensions cannot link `steel-core`: it is an optional dependency behind `host`
  (`steel-extension-api/Cargo.toml:10,14`).

**Decision.** The capability is a host feature: templates, copy-on-write storage and runtime world
lifecycle are platform work in spec 005, and the extension is a thin layer on top.

**Why.** The host already marks this as intended: `steel-core/src/chunk_saver/ram_only.rs:17-18`
says "Will later have the option to load a world from storage and clone it for easy world handling".

## R2. The on-disk format and a read-only reader

**Finding.**

- Steel stores worlds in its own `STLR` region format, not Anvil: magic `STLR`, `FORMAT_VERSION = 26`
  (`steel-core/src/chunk_saver/format.rs:66`), an 8 KB entry table, 4 KB sectors
  (`format.rs:1-29,89-180`), zstd-compressed `wincode` payloads (`region_manager.rs:528-543`).
- Layout: `<save_path>/<domain>/worlds/<name>/region/r.X.Z.srg` plus `level.toml`
  (`chunk_saver/registry.rs:77-81,162-170`, `level_data/mod.rs:423`). Region files are 32×32 chunks.
- Palettes are stored by block key and properties, not state id (`format.rs:295-301,378-381`), and
  resolved on load (`storage/chunk.rs:723-758`), so a template survives registry reordering.
- There is no Anvil reader anywhere in the repo.
- `RegionManager::open_region` opens read+write, creates a missing file, and renames a
  version-mismatched file to `*.srg.v{N}.bak` (`region_manager.rs:102-137`). A chunk that fails to
  decode has its slot deleted (`region_manager.rs:494-511`). None of this is acceptable for template
  sources (FR-003).
- There is no way to list the chunks of a region: the header is private to `RegionHandle`
  (`region_manager.rs:66-77`).
- Header validation (`region_manager.rs:219-260`) is pure and can move to `format.rs`, next to
  `RegionHeader::from_bytes` (`format.rs:244`) and the region file name helper (`format.rs:1219`).
- Per-chunk decoding to the in-memory `Chunk` is `ChunkStorage::try_persistent_to_chunk`
  (`storage/chunk.rs:384-557`, `pub(crate)`). It works with a dead `Weak<World>` (tests:
  `region_manager.rs:772-780`, `storage/tests/chunks_sections.rs:56-62`).

**Decision.** Add a separate strict reader, `chunk_saver/region_reader.rs`:

- It reads each `*.srg` file whole with `tokio::fs::read`, checks magic and version equality, and
  treats any mismatch as an error. It never writes.
- It decodes every chunk on the rayon pool as validation (`PersistentChunk`, then
  `validate_persistent_chunk`, `storage/chunk.rs:395`). Any failure fails the template, naming the
  region and chunk.
- It strips entities there and keeps the compressed payload otherwise untouched.

**Alternative rejected.** Adding a "read-only" mode to `RegionManager`. It would touch the live
save path used by every world and keep the creating and deleting behavior one flag away from
templates.

## R3. Template representation and registry

**Finding.**

- `PersistentChunk` is not `Clone` and borrows string properties (`format.rs:297-301,366-367`).
- `level.toml` holds `WorldGenerationSettings { generator, config, dimension_type, min_y, height }`
  (`level_data/mod.rs:103-121`), checked against the live generator at load (`level_data/mod.rs:457`).
- Game mode is not in `LevelData` (`level_data/mod.rs:70-106`); it comes from `worlds.toml`
  (`config.rs:374,581`).
- `LevelDataManager::new` and the world constructor save when dirty (`world/mod.rs:417-419`), so they
  cannot read a template's `level.toml` without side effects.

**Decision.**

- `WorldTemplate` (new, `steel-core/src/world/template.rs`) is an immutable `Arc` holding the
  generation settings, seed, spawn and difficulty from `level.toml`, and a map from chunk position
  to `{ status, compressed payload }`.
- `LevelData::read_snapshot(path)` (new) parses `level.toml` and game rules with no writes.
- The generator in `level.toml` is validated with `world_generators.validate_config` and compared
  using the same rule as `validate_generation_settings` (`level_data/mod.rs:324`). An unregistered
  generator means Failed.
- The registry (`server/templates.rs`, new) lives on `Server`: key to `Loading | Ready | Failed`.
  Loading is an async task on the existing cleanup tracker (`temporary/mod.rs:155`). Nothing in the
  tick waits for it, and no job is needed because templates are not tick-visible state.
- Storing compressed payloads and decoding per load avoids needing `Clone` and keeps template memory
  small. The decode cost equals a disk load without the I/O.
- The ABI references templates by key (chosen by the caller), not by an opaque handle. The server
  keeps the entry for the process lifetime; each clone also holds an `Arc`.

## R4. Copy-on-write RAM storage

**Finding.**

- `ChunkStorage` is a closed enum `Disk(RegionManager) | RamOnly(RamOnlyStorage)`
  (`storage/mod.rs:450-456`), chosen from `WorldStorageConfig` (`config.rs:196-206`) at
  `world/mod.rs:398-405`. It is not a trait and the factory registry is private
  (`chunk_saver/registry.rs:53`).
- `RamOnlyStorage` keeps a private map of serialized chunks written by `save_chunk_data`
  (`ram_only.rs:19-31,69-78`) and re-materializes on every load through `try_persistent_to_chunk`
  (`ram_only.rs:44-66`).
- The chunk pipeline loads from storage before generating (`chunk/chunk_holder.rs:950-1000`).
- Only dirty chunks are saved (`chunk_map/persistence.rs:41-46`), so an untouched template chunk is
  never copied into a clone.

**Decision.**

- `RamOnlyStorage` gets an optional `Arc<WorldTemplate>` base (`RamOnlyStorage::from_template`).
  `empty_world()` stays.
- Existence checks and loads consult the overlay first, then the template. Loads decode on the
  `thread_pool` that `ChunkStorage::load_chunk` already receives (`storage/mod.rs:474-484`); the RAM
  arm currently ignores it.
- New `WorldStorageConfig::RamClone { template }`. The enum is `Debug + Clone` and not serde, so
  `WorldTemplate` needs a short manual `Debug`.

**Cost.** Chunks near players get dirtied (ticking, light, inhabited time), so sharing mostly helps
for chunks nobody visits. Overlay entries are uncompressed `PersistentChunk`s; zstd-compressing them
is a possible follow-up, not part of this spec.

## R5. Runtime world requests and independent clocks

**Finding.**

- `TemporaryWorldRequest` (`temporary/mod.rs:27-44`) has `primary: bool`. A non-primary world must
  join a temporary domain (`mod.rs:140-149`) whose primary is published (`mod.rs:110-114`).
- Non-primary worlds use `GameTimeSource::Derived` from the primary (`mod.rs:121-124`), and
  `publish_temporary_world` rejects any member that does not share the primary's clock
  (`server/worlds.rs:143-146`).
- `advance_domain_game_times` advances only each domain's default world and panics if that world is
  missing (`worlds.rs:91-99`).
- A world without a level-data path starts with `game_time: Some(0)` and fresh clocks
  (`level_data/mod.rs:303-317,476-486`), which is what `GameTimeSource::Primary` already gives.

**Decision.**

- Generalize the request with `WorldSource { Generator{generator, config}, Template }`,
  `DomainTarget { NewTemporary, Existing }` and a clock choice. A clone and a generated runtime
  world use `GameTimeSource::Primary`, so no new clock variant is needed.
- `WorldMapState` replaces `default_worlds` with `DomainEntry { default_world: Option<Identifier> }`
  and adds `runtime_worlds` and `clock_owners`. `advance_domain_game_times` iterates `clock_owners`.
  `publish_temporary_world` becomes `publish_runtime_world` with explicit placement.
- Older derived siblings keep their behavior (FR-029); a derived member must still share its
  primary's clock.
- The `has_domain` check (`worlds.rs:225`) must survive removal of a domain's default world, because
  `select_world`, teardown and the lifecycle code call it (`world_changes.rs:412`).

## R6. Admission to lasting domains, and the saved-data bug

**Finding.**

- Today only temporary domains accept members, because `state.domains` is filled only by primary
  temporary creation (`temporary/mod.rs:128-139`).
- Scoreboard and command storage saves pick the world to write through by asking whether it owns a
  clock: `scoreboard.rs:564-588` (`owns_game_time()` at `:569`) and `command/storage.rs:188-212`.
- A world without a storage path makes `SavedDataManager::save` return `Ok(())` without writing
  (`steel-utils/src/saved_data.rs:216-218`), and the caller then calls `mark_saved`
  (`scoreboard.rs:585`).

**Consequence.** A clone with its own clock inside a lasting domain would pass `owns_game_time()`,
and the domain's pending scoreboard and command storage changes would be marked saved without being
written. This is a silent data-loss bug introduced by independent clocks, and it is fixed
foundationally (FR-015).

**Decision.**

- Pick the save target with `worlds.default_world(domain)` and skip domains that have none. This is
  also what `DomainRandomSequences::save` already does (`loot/random_sequences.rs:229-252`).
- `DomainTarget::Existing` accepts a lasting domain when `worlds.has_domain(domain)` and the domain
  is not temporary (`player_data_storage.is_temporary_domain`). Duplicate keys are rejected at
  request time (loaded or pending), not only at publication (`worlds.rs:124-126`).
- A per-domain `creating` count also covers lasting domains, so removal and creation order correctly.
- Entity directory: `insert_into` (`worlds.rs:105-117`) already joins the domain directory; removal
  calls `directory.remove_world` (`world/domain_entity_directory.rs:35`).

## R7. Loot random sequences

**Finding.**

- Lasting domains hold one `RandomSequences` per domain in `DomainRandomSequences`, loaded at
  startup and shared by all of the domain's worlds (`loot/random_sequences.rs:197-253`).
- A new temporary primary creates its own `RandomSequences::new(seed)`, and a member copies the
  primary's (`temporary/mod.rs:105-125`), failing when the domain has no default world
  ("temporary domain is not loaded", `mod.rs:111-114`).

**Decision (D8).** A clone behaves as a `worlds.toml` world: it uses its domain's sequences. For a
lasting domain that is the existing entry in `DomainRandomSequences`, which needs an accessor from
`Server`. For a new temporary domain the clone is the primary and creates fresh sequences.

**Open point (A4).** After a temporary domain loses its first world, joining it again would fail
at `mod.rs:111-114`, because the sequences live on the world. The proposed fix is to hold the
sequences on `DomainEntry` for temporary domains too. This contradicts nothing in the decisions but
adds a field, so it is flagged for confirmation.

## R8. Removing a single world

**Finding.**

- There is no single-world removal: `WorldMap` has only `insert`, `publish_temporary_world` and
  `remove_temporary_domain` (`worlds.rs:101-172`).
- Domain teardown is the pattern: `DestroyDomainJob` (`temporary/mod.rs:314-378`) waits while a
  creation is in flight, any player is inside, any admission is `Relocating` or `Disconnecting`
  (server-wide, `:331-336`), or a domain switch targets the domain; it then removes worlds, fires
  `world_unload`, retires player routing and stops each world's chunk tasks asynchronously
  (`mod.rs:346-376`).
- Entry points that must refuse a retiring world: `queue_player_world_selection`
  (`world_changes.rs:397-411`), `start_domain_switch` (`world_changes.rs:507-526`),
  `WorldSpawnTeleportJob` revalidation (`jobs/teleport.rs:261-264`), portal resolution
  (`worlds.rs:257-296`), and runtime-world member placement.
- A joining player is finalized in `finish_prepared_player_join` (`player_admission.rs:248+`).

**Decision.**

- `Server::remove_world(key)` (new, `server/temporary/removal.rs`) fails for keys outside
  `runtime_worlds`, worlds already being removed, closing domains, worlds still being created, and
  clock owners that loaded derived worlds share (A3).
- New `World::retiring: AtomicBool` is set at start and checked at all entry points above.
- `RemoveWorldJob::poll` runs in the job stage. Each poll queues
  `queue_player_world_selection(player, target, WorldEntry::Usual)` for every player still inside,
  where `target` is the domain default world if it exists, is not this world and is not retiring,
  otherwise `server_default_world()`. Transient errors (dead player, credits screen, already
  relocating) retry on the next poll. It stays pending while players remain, a domain switch or world
  change targets the world, or any admission is `Relocating`/`Disconnecting`.
- When clear it removes the world from the map, `runtime_worlds`, `clock_owners`, the entity
  directory and portal maps, clears `default_world` if it was the default (keeping the domain), fires
  `world_unload`, and stops the chunk map. The stop logic at `temporary/mod.rs:366-372` moves into a
  shared `World::retire_runtime`.
- `DomainState` gets a `removing` counter, and `DestroyDomainJob` waits for it to reach 0.
- `finish_prepared_player_join` re-checks that the target world is still registered and not
  retiring, and disconnects cleanly otherwise (defensive; reconnect never targets clones).
- Without a default world, `switch_domain` fails at `server/player_lifecycle.rs:69-72`
  ("has no default world"), and `select_world` into a remaining world still works (A1).

## R9. Reconnect, respawn and portals

**Finding.**

- A saved world is restored by `resolve_saved_world` (`player_lifecycle.rs:212-245`): it falls back
  to the target spawn when the key is missing (`:238-242`) or outside the target domain
  (`:227-232`), and both join and domain switch go through it.
- Respawn handling uses the domain default world (`player_lifecycle.rs:517-564`,
  `player/lifecycle/respawn.rs:98-131`); a temporary domain without a default fails at
  `player_lifecycle.rs:525-533`.
- Portal targets resolve by naming convention (`worlds.rs:257-296`).

**Decision.**

- `resolve_saved_world` returns `None` (with a log line) for a runtime world of a lasting domain,
  giving `SavedWithoutLocation` at the domain default with inventory kept (`:82-89`). This one place
  covers joins and switches.
- Respawn adds only the server-default fallback when a temporary domain has no default (D10). Beds
  and saved ender pearls keep their current handling, which already degrades to a spawn when the key
  is missing (`:376`).
- `resolve_*_portal_target` returns `None` when the source or target is a runtime world (D11).

## R10. Risks and open points

1. **Overlay memory** (R4): sharing helps unvisited chunks mostly. A follow-up could compress overlays.
2. **Seed override with a real generator**: unsaved chunks near template edges would generate
   differently from the template's seed. With `steel:empty` this is invisible; for other generators
   the host should warn, or reject an override. Needs a decision before US1 is finalized.
3. **Shutdown cost**: `World::cleanup` serializes RAM worlds for nothing. Existing issue, not fixed here.
4. **A4** (R7): random sequences held by the domain for temporary domains.
5. **Operation leaks**: unread operations stay in the host table forever, so every caller must poll
   what it receives. Documented in [contracts/host-api.md](contracts/host-api.md).
