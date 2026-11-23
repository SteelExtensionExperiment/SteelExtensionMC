# Data Model: Runtime RAM Worlds from Saved Templates (host platform)

Entities are marked **(new)**, **(existing)** or **(extended)**. Paths are relative to the Steel
checkout. All changes are platform gaps, tracked in [plan.md](plan.md). The extension that uses them has its own data model in `extension-example/world-cloner/specs/001-world-cloner/data-model.md`.

## Platform: host core (`steel-core`)

### `RegionReader` (new) — `chunk_saver/region_reader.rs`

Strict read-only reader for a world's `region/` directory.

| Item | Rule |
| --- | --- |
| Input | `read_all(region_dir, min_y, height, pool)`: the region directory, the world's build range for `validate_persistent_chunk`, and the rayon decode pool. A missing directory yields no chunks and is not created. |
| Files | Every `r.<x>.<z>.srg`. Other files are ignored with a debug log. |
| Header | Magic `STLR` and `FORMAT_VERSION` must match; otherwise an error. Entry validation is shared with `RegionManager` through `RegionHeader::validate_entries` and `ChunkEntry::validate` (moved into `format.rs`). |
| Per chunk | zstd, then `wincode` to `PersistentChunk`, then `validate_persistent_chunk(min_y, height)`. Entities are cleared and the payload re-encoded only when entities were present. |
| Output | `Vec<(ChunkPos, ChunkStatus, Box<[u8]>)>` or an `io::Error` naming the region and chunk. |
| Side effects | None. Never creates, renames, truncates or deletes. |

`RegionPos::from_filename` (new, `format.rs`) is the inverse of the existing file name helper and
accepts negative coordinates.

### `WorldTemplate` (new) — `world/template.rs`

Immutable, shared as `Arc<WorldTemplate>`.

| Field | Meaning |
| --- | --- |
| `key` | `Identifier`, chosen by the caller (the world-cloner extension uses `world-cloner:<name>`). |
| `source` | Source folder path (diagnostics only). |
| `generation` | `WorldGenerationSettings` from `level.toml` (generator, config, dimension type, `min_y`, `height`). Required; absent means Failed. The saved generator must be registered (`validate_config`) and its dimension type, `min_y` and `height` must equal the saved ones; the config itself is taken as saved. |
| `seed`, `spawn`, `spawn_angle`, `difficulty` | Defaults for clones. |
| `chunks` | `FxHashMap<ChunkPos, TemplateChunk { status: ChunkStatus, payload: Box<[u8]> }>`; payloads are the compressed `PersistentChunk` bytes without entities. |
| Methods | `chunk(pos)`, `memory_bytes()`. A short manual `Debug`. |

Not carried over: game rules, world border, weather, tickets, `data/` (decision D7).

### `LevelData::read_snapshot` (new) — `level_data/mod.rs`

`read_snapshot(path) -> io::Result<LevelData>` parses `level.toml` (and the game rules loader) with no
writes. The existing `LevelDataManager::new` and the world constructor save when dirty and cannot be
reused (`world/mod.rs:417-419`).

### `TemplateEntry` and the registry (new) — `server/templates.rs`

```text
WorldTemplates { entries: SyncMutex<FxHashMap<Identifier, TemplateEntry>> }
TemplateEntry  = Loading | Ready(Arc<WorldTemplate>) | Failed(String)
```

| Transition | Rule |
| --- | --- |
| absent → Loading | `Server::load_world_template(key, path)`; fails if the key exists in any state. |
| Loading → Ready / Failed | The load task publishes under the lock, then finishes its operation. |
| Ready, Failed | Terminal for the process lifetime. A retry needs a new key. |

### `WorldOperations` (existing, moved) — `server/world_operations.rs`

`WorldOperationId`, `allocate`, `finish` and `take_world_operation` move out of
`server/temporary/mod.rs:23-75,178-193`. Create, destroy, template load and remove share one table.
Semantics are unchanged: one server-wide table, results consumed on first read, ids fail on overflow.

### `RamOnlyStorage` (extended) — `chunk_saver/ram_only.rs`

| Field | Meaning |
| --- | --- |
| `saved_chunks` (existing) | Overlay: chunks this world saved. |
| `base: Option<Arc<WorldTemplate>>` (new) | Read-through source for chunks the overlay lacks. |

| Operation | Behavior |
| --- | --- |
| exists / acquire | True if in the overlay or the template. |
| load | Overlay first; otherwise decode the template payload on the pool and call `try_persistent_to_chunk` with the template's status. |
| save | Overlay only. |

### `WorldStorageConfig` (extended) — `config.rs`

New variant `RamClone { template: Arc<WorldTemplate> }`, matched in `world/mod.rs:398-405`.

### `RuntimeWorldRequest` (extended from `TemporaryWorldRequest`) — `server/temporary/mod.rs`

| Field | Values |
| --- | --- |
| `key` | `domain:world` (existing). |
| `source` | `Generator { generator, config }` or `Template(Arc<WorldTemplate>)` (new). |
| `target` | `NewTemporary` (becomes the old `primary: true`) or `Existing` (new; lasting or temporary domain). |
| `seed`, `spawn`, `default_game_mode`, `difficulty` | Existing; for templates they are resolved from the template and the overrides before the request is built. |

Rules: spawn is within the dimension's build height; the key is not loaded and not pending
(checked at request time); `Existing` requires a loaded domain; `NewTemporary` requires an unused
domain identity (existing rule, `mod.rs:94-96`).

### `WorldMapState` (extended) — `server/worlds.rs`

| Field | Change |
| --- | --- |
| `default_worlds: Map<String, Identifier>` | Replaced by `domains: Map<String, DomainEntry { default_world: Option<Identifier>, random_sequences: Option<Arc<RandomSequences>> }>`. The sequences field is assumption A4. |
| `runtime_worlds: Set<Identifier>` | New. Every world that can be removed. |
| `clock_owners: Vec<Identifier>` | New. `advance_domain_game_times` advances these instead of reading default worlds. |
| `has_domain` | Reads `domains`, so it stays true after the default world is removed. |

`publish_runtime_world(world, RuntimePlacement { NewDomainDefault | Member })` replaces
`publish_temporary_world`. A member that owns its clock is allowed; a derived member must still match
its primary's clock (`ptr_eq`), so older siblings keep their behavior.

### `World` (extended) — `world/mod.rs`

New `retiring: AtomicBool`, set when removal starts. New `World::retire_runtime(self: Arc<Self>)`
holds the stop logic now at `server/temporary/mod.rs:366-372`.

### `DomainState` (extended) — `server/temporary/mod.rs`

New `removing: usize`. `DestroyDomainJob` waits for it to reach 0. The existing `creating` count also
covers lasting domains that receive clones.

### `RemoveWorldJob` (new) — `server/temporary/removal.rs`

| Step per poll | Rule |
| --- | --- |
| Relocate | Queue a world selection for each player still inside, to the domain default (if present, not this world, not retiring) or the server default world. Retry transient errors next poll. |
| Wait | Pending while players remain, a world change or domain switch targets the world, or any admission is `Relocating`/`Disconnecting`. |
| Remove | Drop from the map, `runtime_worlds`, `clock_owners`, entity directory and portal links; clear `default_world` if needed; fire `world_unload`; stop chunk tasks. |
| Result | The removed world's key. |

### `DomainRandomSequences` (extended) — `loot/random_sequences.rs`

Gains an accessor so `Server` can hand a lasting domain's shared sequences to a new clone. No change
to saving (`save` already uses the domain default world).

## Platform: extension API (`steel-extension-api`)

All types are `#[stabby::stabby]`. Existing items and `create_temporary_world` are unchanged.

### `runtime/types.rs` (extended)

| Type | Shape |
| --- | --- |
| `TemplateState` | `Loading`, `Ready`, `Failed` (`repr(u8)`). |
| `TemplateStatus` | `{ state: TemplateState, message: ApiString, chunks: u64 }`; `message` is empty unless Failed. |
| `CloneTarget` | `NewTemporaryDomain`, `ExistingDomain`. |
| `CloneSettings` | `{ seed: StableOption<i64>, spawn: StableOption<BlockPos>, default_game_mode: StableOption<GameMode>, difficulty: StableOption<Difficulty> }`. |

### `Server<'_>` methods (new) — `runtime/bindings.rs`, adapters in `host/runtime.rs`

See [contracts/host-api.md](contracts/host-api.md).
