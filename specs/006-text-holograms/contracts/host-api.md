# Contract: host bindings (`steel-extension-api`)

Proposed methods on `Server<'_>` in `steel-extension-api/src/runtime/bindings.rs`, with adapters in
`host/runtime.rs`, in the style of the existing bindings. Existing bindings keep their behavior (FR-008).
All methods return `ApiResult`, never block, and fail with a message.

| Method | Result | Notes |
| --- | --- | --- |
| `create_hologram(world: Str, position: Position, text: Str, billboard: Billboard)` | `ApiResult<HologramId>` | Stores the record; the entity appears now if the chunk is loaded, otherwise on chunk load. Fails for an unloaded world or a position outside the build height. Text never fails: malformed JSON is plain text, as in every other text API. |
| `set_hologram_text(id: HologramId, text: Str)` | `ApiResult<()>` | One entity-data update if the entity exists; otherwise stores the text for the next load. |
| `remove_hologram(id: HologramId)` | `ApiResult<bool>` | `true` if it existed. Removes the entity if loaded. |

## Rules

- `world` is a `domain:world` key as elsewhere in `Server` bindings.
- `text` is plain text or a vanilla JSON component (`steel-extension-api/src/text.rs:1-6`); multiple lines
  are newlines in the text.
- A handle is invalid after `remove_hologram` or after its world unloads; operations on it fail with a
  message, except `remove_hologram`, which returns `false`.
- An extension that wants its holograms back after a world load recreates them in its own
  `on_world_load` handler; the host does not remember them across world unloads.
