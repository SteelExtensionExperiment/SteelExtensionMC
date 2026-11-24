# Contract: host bindings (`steel-extension-api`)

New methods on `Server<'_>` in `steel-extension-api/src/runtime/bindings.rs`, with adapters in
`host/runtime.rs`. Existing bindings, including `create_temporary_world` and
`destroy_temporary_domain`, keep their behavior (FR-026).

The `world-cloner` extension wraps these in its own versioned service; that contract is specified in
`extension-example/world-cloner/specs/001-world-cloner/contracts/service.md`.

All methods return `ApiResult` and never block. Operations complete on a later tick and are polled with
the existing `Server::operation_status`, which consumes the result on the first read.

| Method | Result | Operation value on success |
| --- | --- | --- |
| `load_world_template(key: Str, path: Str)` | `ApiResult<Operation>` | The template key. |
| `world_template_status(key: Str)` | `StableOption<TemplateStatus>` | — |
| `clone_world(template: Str, key: Str, target: CloneTarget, settings: CloneSettings)` | `ApiResult<Operation>` | The loaded world's key. |
| `create_world(key: Str, generator: Str, target: CloneTarget, settings: WorldSettings)` | `ApiResult<Operation>` | The loaded world's key. |
| `remove_world(key: Str)` | `ApiResult<Operation>` | The removed world's key. |
| `world_is_removable(key: Str)` | `bool` | — |

## Types (`runtime/types.rs`, all `#[stabby::stabby]`)

| Type | Shape |
| --- | --- |
| `TemplateState` | `Loading`, `Ready`, `Failed` (`repr(u8)`). |
| `TemplateStatus` | `{ state: TemplateState, message: ApiString, chunks: u64 }`; `message` is empty unless Failed. |
| `CloneTarget` | `NewTemporaryDomain`, `ExistingDomain`. |
| `CloneSettings` | `{ seed: StableOption<i64>, spawn: StableOption<BlockPos>, default_game_mode: StableOption<GameMode>, difficulty: StableOption<Difficulty> }`. |

## `load_world_template`

- `key` is an identifier (the extension chooses it). `path` is an absolute directory holding `region/`
  and `level.toml`.
- Fails immediately when the key already has an entry in any state, or the path is not a directory.
- The operation fails with the diagnostic when the template cannot be loaded; `world_template_status`
  then shows `Failed` with the same message. See [research R2](../research.md#r2-the-on-disk-format-and-a-read-only-reader).

## `clone_world`

- `target = NewTemporaryDomain`: the key's namespace is a new temporary domain name that has never
  been used; the clone becomes the domain's default world and its random sequences are created fresh.
- `target = ExistingDomain`: the key's namespace is a loaded domain, lasting or temporary and not
  closing; the clone joins it and uses the domain's random sequences. A temporary domain whose
  first world was removed has no default world and no sequences to share, so this fails with a
  clear message (decision A4, rejected by the user).
- A `Some` `settings.seed` fails immediately unless the template's generator is `steel:empty`, so a
  clone never generates unsaved chunks from a different seed than its saved ones (decision R10.2).
- Fails immediately (no operation) when the template is not Ready, the key is loaded or pending, the
  domain rules above are not met, or the resolved spawn is outside the dimension's build height.
- Defaults come from the template (seed, spawn, difficulty) and every `Some` field of `settings`
  overrides one. Game mode is not stored in the template, so a `None` game mode uses the server's
  default for new worlds.
- The clone has its own clock starting at 0, never a derived one.

## `create_world`

- Same targets and key rules as `clone_world`, with a registered generator and its default
  configuration, as `create_temporary_world` uses today. A generator that needs configuration, such as
  `steel:empty` (`worldgen/generator/registry.rs:272-275,364-371`), fails validation with a message.
- `create_temporary_world` remains for existing callers and keeps its derived-clock siblings. New code
  uses `create_world`.

## `remove_world`

- Allowed only for runtime worlds (`world_is_removable` is true). Fails immediately for `worlds.toml`
  worlds, worlds still being created, worlds already being removed, closing domains, and clock owners
  that older derived worlds share.
- From the moment it succeeds, the world refuses new entries. Players inside are relocated by the host
  (default world of the domain, else the server default world). The operation completes when the world
  is retired.
- The domain stays registered. A temporary domain left with no default world refuses `switch_domain`;
  `select_world` into its remaining worlds works.

## Operation rules

- A returned `Operation` belongs to its caller. Poll it until `Ready` or `Failed`; an unread result
  stays in the host table for the life of the process.
- `OperationStatus::value` is empty while pending, the key on success, or a diagnostic on failure.
