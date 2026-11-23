# Data Model: Extension-controlled Text Holograms (host platform)

All items are **(new)** unless stated. Names are proposals; spelling may change to fit the code.

## Host (`steel-core`)

| Item | Shape |
| --- | --- |
| `Hologram` | `{ position: DVec3, text: TextComponent, billboard: BillboardConstraints, entity: Option<Weak<TextDisplayEntity>> }`. No owner and no world field: the record lives in its world's registry. `entity` is `None` while its chunk is unloaded. |
| `HologramId` | Opaque `u64` from one process-wide `AtomicU64` counter, so ids are unique across worlds and never reused in a process. |
| Hologram registry | A field of `World` (dropped with the world), behind one `SyncMutex`. `holograms: HashMap<HologramId, Hologram>`, `by_chunk: HashMap<ChunkPos, Vec<HologramId>>`. `World` methods: `create_hologram`, `set_hologram_text`, `remove_hologram`; chunk load and unload hooks in `entity_management.rs`. Lookup across worlds is done by the caller scanning worlds. |
| Non-persistent text display | A text display whose `is_persistent` is false (R4). A flag on `TextDisplayEntity` set only by `TextDisplayEntity::new_hologram` (T004). |

No hologram data is ever serialized (H2).

## Extension API (`steel-extension-api/src/runtime/types.rs`, `#[stabby::stabby]`)

| Type | Shape |
| --- | --- |
| `HologramId` | `{ id: u64 }`, `Copy`, opaque (field not public). |
| `Billboard` | `Fixed`, `Vertical`, `Horizontal`, `Center` (`repr(u8)`), mapping to the vanilla constraints. |

## Rules

- A hologram has at most one live entity; it exists only while its chunk is loaded (H3).
- `create` stores the record even when its chunk is not loaded; the entity appears on chunk load.
- Removing a world drops its registry and entities with it (H4).
