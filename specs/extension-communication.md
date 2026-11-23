# Extension communication: Vault pattern and hook actions

## Context

You plan four extensions (world-cloner, arena-manager, stats-visualizer, game-orchestrator) that must
talk to each other. Their specs leave the connection open (O2 in arena-manager and game-orchestrator, D9/D12).
Decided: services go down, hook actions come up (option A in section 3); no `game-registry`; the orchestrator re-emits results for stats-visualizer. Options B-D are kept for reference.

---

## 1. How Vault does it

### Three pieces

| Piece | File | Role |
| --- | --- | --- |
| Host service registry | `steel-extension-api/src/services.rs` | Key → leaked `&'static` object + stabby `TypeReport`. Like Bukkit `ServicesManager`. |
| Contract crate | `extension-example/vault-api/src/lib.rs` | Plain library (not an extension). Keys, stabby traits, payload types, `economy()`/`permission()`/`chat()` getters. |
| Provider extension | `extension-example/vault/src/lib.rs` | Implements `Permission` + `Chat`, registers them in `init`. |

### Contract crate (`vault-api`)
- Versioned keys: `"vault:economy/v1"`, `"vault:permission/v1"`, `"vault:chat/v1"`. Change suffix on incompatible change.
- Traits are `#[stabby::stabby(checked)] pub trait Economy: Send + Sync { extern "C" fn ... }`. Only ABI-stable
  types cross the boundary: `stabby::string::String`, `Str<'_>`, `stabby::vec::Vec`, `Uuid`, `#[stabby::stabby]` structs/`#[repr(u8)]` enums (`EconomyResponse`, `ResponseKind`).
- Methods needing host state take a borrowed `Server<'_>` (permission/chat), because provider has no host handle of its own.
- Owning object type: `pub type DynEconomy = stabby::dynptr!(stabby::boxed::Box<dyn Economy + Send + Sync>);`
- Getter: `pub fn economy() -> Result<&'static DynEconomy, String> { services::get(ECONOMY) }`.

### Provider (`vault`)
- `struct SteelPermission;` + `#[stabby::export(callback)] impl Permission for SteelPermission { ... }`.
- In `init`: `services::register::<DynPermission>(&context, PERMISSION, 0, Box::new(SteelPermission).into())?`.
  Priority 0; highest priority wins lookups (Essentials registers economy at 10).
- `pre_init` only registers `/vault-info`, which lists providers via `services::providers(key)`.

### Consumers (`essentials`, `prison-enchants`)
- `Cargo.toml`: `vault-api = { path = "../vault-api" }` (sibling path, never copied).
- `export_extension!{ soft_depends: ["vault", "essentials"] }` → load/init order only; missing provider is tolerated.
- Call site: `use vault_api::EconomyDyn as _;` then `vault_api::economy()?.deposit(uuid, 25.0)` **at use time**, not cached at init.

### Safety mechanism
`services::get::<T>` compares `T::REPORT` against the provider's stored report; layout mismatch → `Err("incompatible provider ...")`
instead of UB. Objects are leaked and libraries never unload, so `&'static` is sound.

### Limits that matter for your four extensions
1. **One-way.** Consumer pulls from provider. Provider cannot call consumer unless consumer also exposes something.
2. **No cycles.** Host rejects dependency cycles, soft ones too (`steel-extension-api/src/host/lifecycle.rs:201-239`, test at :366).
3. `services::get` returns only the highest-priority provider; `providers()` returns names, not objects.
4. Registration happens in `init`; `depends` only guarantees the provider's `init` ran before yours. Lookups at
   use time (commands, ticks) don't strictly need `depends` — it documents intent and fails fast if missing.

---

## 2. Target topology (with your decisions)

```text
world-cloner ──provides── world-cloner:clone/v1           (world-cloner-api)
     ▲ depends
arena-manager ──provides── arena-manager:match/v1         (arena-manager-api)
     ▲ depends                                 │ results (see §3)
game-orchestrator ──provides── game-orchestrator:games/v1 (game-orchestrator-api)
     ▲ depends                                 │ re-emits results
stats-visualizer (no api crate)
```

- Dependency edges all point one way → no cycle, no `game-registry`.
- Orchestrator owns game types + kits; arena-manager treats game type as opaque `ApiString`, receives
  a kit per player inside `start_match`. Arena-manager never links `game-orchestrator-api`.
- Stats links only `game-orchestrator-api`: `game_types()` via service, results via whatever §3 picks.

### Services in vault style (the "down" direction, no choice needed)

| Crate | Key | Methods (draft) |
| --- | --- | --- |
| `world-cloner-api` | `world-cloner:clone/v1` | `templates`, `clone_world`, `create_world`, `remove_world` → `ApiResult<Operation>` (already in its contract) |
| `arena-manager-api` | `arena-manager:match/v1` | `start_match(server, StartRequest) -> ApiResult<MatchId>`, `matches(server) -> ApiVec<MatchInfo>`, `game_types_with_arenas(server)` |
| `game-orchestrator-api` | `game-orchestrator:games/v1` | `game_types(server) -> ApiVec<ApiString>` |

`StartRequest { game_type: ApiString, players: ApiVec<PlayerKit> }`, `PlayerKit { uuid, kit: Kit }` — kit
type defined in `arena-manager-api` because arena-manager consumes it (provider owns contract).

---

## 3. Result delivery ("up" direction) — options to decide

Same question applies twice: arena-manager → orchestrator, orchestrator → stats. Pick one mechanism for both.

### A. Hooks Action (event bus already in the API)
- `arena-manager-api`: `#[stabby::stabby] pub struct MatchFinished { match_id, game_type, participants: ApiVec<Participant> }`,
  `impl Action for MatchFinished { const TAG = "arena-manager:match_finished/v1"; }`. `Outcome` enum `Won|Lost|Draw` + `MatchFailed { match_id, message }`.
- Emit: `hooks::do_action(&MatchFinished { .. })`. Subscribe in `pre_init`: `hooks.add_action::<MatchFinished>(on_finished).register()?`.
- Reference: `extension-example/hooks-and-filters/{contract,base}/src/lib.rs`, `steel-extension-api/src/hooks/mod.rs`.
- **Pros:** built-in, zero new code in host; any number of listeners; no `depends` needed by listeners → never a cycle; ordered with `.before/.after`.
- **Cons:** callback is `extern "C" fn(&A)` — no captured state, listener must keep state in a `static` (`OnceLock<SyncMutex<..>>`).
  Synchronous, nested dispatch: emitter must **drop its own locks before `do_action`**, else a listener that calls back
  (`start_match` for next queue) deadlocks. Handlers only registrable in `pre_init`.

### B. Inverse service (listener registers a service the emitter looks up)
- `arena-manager-api` defines `trait MatchListener { extern "C" fn on_finished(&self, server: Server<'_>, result: &MatchFinished); }`
  and key `arena-manager:listener/v1`. Orchestrator registers its impl with `services::register`; arena-manager calls
  `services::get::<DynMatchListener>(LISTENER)` when a match ends (lazy, so no `depends` → no cycle).
- **Pros:** pure vault pattern, no new concepts; listener gets `Server<'_>` and owns state in `self`.
- **Cons:** only **one** listener per key (highest priority) — stats can't also listen; fine here since orchestrator
  re-emits. Same reentrancy/lock rule as A. Abuses "provider owns contract" slightly (consumer implements it).

### C. Registration method on the provider's service
- `MatchService::add_listener(&self, listener: DynMatchListener)`; arena-manager stores listeners in a `SyncMutex<Vec<_>>`.
  Orchestrator calls it in `init` (has `depends: ["arena-manager"]` already).
- **Pros:** many listeners, stateful objects, explicit ownership.
- **Cons:** most code: passing owned stabby trait objects across the boundary, storing them, lifetime rules
  (libraries never unload, so OK but must be documented). Every intermediate (orchestrator) repeats it for stats.

### D. Pull / poll (like world-cloner `Operation`)
- `start_match` returns `MatchId`; orchestrator polls `match_status(id)` or `drain_results()` each tick.
- **Pros:** no callbacks, no reentrancy, matches `Server::operation_status` style; deterministic tick order.
- **Cons:** one-consumer drain semantics (or per-consumer cursors), polling every tick, result latency of a tick,
  more state in arena-manager (results kept until read). Stats would poll orchestrator the same way.

### My recommendation
**A (Actions)** for both hops: least code, many listeners, no cycles, already proven in an example.
Use **D** only if you want zero reentrancy risk. B is a good fallback if Action's stateless callbacks feel too awkward.

---

## 4. Integration checklist per extension (once you choose)

1. Create `<name>-api/` sibling crate like `vault-api/Cargo.toml` (own `[workspace]`, dep `steel-extension-api = { path = "../../steel-extension-api" }`, same lint block, `#![recursion_limit = "256"]`).
2. Put key consts, stabby traits, `Dyn*` alias, getter fn, payload/Action types there.
3. Extension crate: `crate-type = ["cdylib", "rlib"]`, dep on own api + providers' api by sibling path, `extension.toml` with `[package]`/`[targets]` only.
4. Provider: `#[stabby::export(callback)] impl Trait for X`, `services::register` in `init`.
5. Consumer: `depends`/`soft_depends` in `export_extension!` long form, `use x_api::TraitDyn as _;`, look up service at use time.
6. Update each `specs/001-*/contracts/service.md` to close O2, remove `game-registry` from arena-manager D9, update `specs/extensions-workflow.md` dependency graph.

## Verification (when implemented later)
- Build each crate: `cargo build --manifest-path extension-example/<crate>/Cargo.toml`.
- Native test pattern: `steel-extension-api/tests/` (`ports`, `prison`, `events` tests load built `.so`s); add one that loads
  arena-manager + orchestrator, starts a match, finishes it, asserts orchestrator and stats saw one result.
- Manual: `/vault-info`-style `/orchestrator status` should show providers resolved.
