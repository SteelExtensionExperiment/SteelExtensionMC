# Data Model: Extension-controlled Text Holograms (host platform)

All items are **(new)** unless stated. Names are proposals; spelling may change to fit the code.

## Host (`steel-core`)

| Item | Shape |
| --- | --- |
| `Hologram` | `{ id: HologramId, owner: ExtensionId, world: WorldKey, position: DVec3, text: String, billboard: BillboardConstraints, entity: Option<Weak<TextDisplayEntity>> }`. `entity` is `None` while its chunk is unloaded. |
| `HologramId` | Opaque `u64`, assigned by the registry from a counter, never reused in a process. |
| `HologramRegistry` | Behind one `SyncMutex`. `holograms: HashMap<HologramId, Hologram>`, `by_chunk: HashMap<(WorldKey, ChunkPos), Vec<HologramId>>`. Methods: `create`, `set_text`, `remove`, `on_chunk_load`, `on_chunk_unload`, `on_world_unload`. |
| Non-persistent text display | A text display whose `is_persistent` is false (R4). Mechanism decided in T002. |

No hologram data is ever serialized (H2).

## Extension API (`steel-extension-api/src/runtime/types.rs`, `#[stabby::stabby]`)

| Type | Shape |
| --- | --- |
| `HologramId` | `{ id: u64 }`, `Copy`, opaque (field not public). |
| `Billboard` | `Fixed`, `Vertical`, `Horizontal`, `Center` (`repr(u8)`), mapping to the vanilla constraints. |

## Rules

- A hologram has at most one live entity; it exists only while its chunk is loaded (H3).
- `create` stores the record even when its chunk is not loaded; the entity appears on chunk load.
- `on_world_unload` removes the entities and the records of that world (H4).
- Operations check the caller's extension against `owner` (H6).
