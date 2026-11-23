# Implementation Plan: Runtime RAM Worlds from Saved Templates (host platform)

**Branch**: `005-world-templates` | **Date**: 2026-11-23 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/005-world-templates/spec.md`

## Summary

Add one reusable platform capability to SteelExtensionMC. The `world-cloner` extension
(`extension-example/world-cloner/specs/001-world-cloner/`) is its first consumer; the arena manager and game
orchestrator reach it through that extension's service.

1. **Runtime worlds from saved templates** ([research R2–R8](research.md)):
   - A strict read-only reader for Steel region files, and an immutable in-memory `WorldTemplate`
     with a registry on `Server`.
   - Copy-on-write RAM storage: clones share the template and keep only chunks they save.
   - A generalized world creation request: template or generator source, new temporary domain or
     existing (lasting or temporary) domain, independent clock.
   - Removal of a single runtime world with host-side relocation of players.
   - Fixes needed so lasting domains stay correct with runtime worlds in them (saved scoreboard and
     command storage, reconnect, portals, respawn).
2. **Extension API bindings** ([contracts/host-api.md](contracts/host-api.md)): stabby types and
   `Server<'_>` methods to load templates, clone, create, remove and query status.

## Technical Context

**Language/Version**: Rust (workspace `rust-toolchain.toml`, nightly with `-Zthreads=0`)

**Primary Dependencies**: existing only: `stabby`, `steel-core`, `steel-registry`,
`steel-extension-api`. No new crates.

**Storage**: none new on disk. Templates are read from existing Steel world folders; clones are RAM-only.

**Testing**: `cargo test` unit tests in `steel-core`, plus a native integration test
`steel-extension-api/tests/world_templates.rs` that drives a small probe extension kept in the Steel
repo, so the platform is testable without any extension.

**Target Platform**: Linux server, vanilla Java clients.

**Project Type**: server platform (host crates).

**Performance Goals**: no tick waits on loading or removal; unvisited clone chunks cost no memory
(proposed targets SC-001 to SC-003, to be confirmed).

**Constraints**:
- Byte-identical host behavior without callers of the new bindings (FR-026).
- Template folders are never written.
- No cap on clone count.

**Scale/Scope**: templates of tens to a few hundred chunks per arena; tens to hundreds of clones.

## Constitution Check

| Principle | Status | How |
| --- | --- | --- |
| I. Stable Native Boundary | Pass | Every new API type is a stabby type; new methods extend `Server<'_>` under the existing runtime module. Existing keys and methods are unchanged. |
| II. Modding-Grade Extensibility | Pass | World sources and targets are enums, not hardcoded worlds. `ChunkStorage` stays a closed enum (existing TODO at `storage/mod.rs:450`); the new `RamClone` variant follows that enum. |
| III. Vanilla Parity First | Pass | No gameplay change without a caller. Templates and clones reuse the vanilla chunk pipeline and the existing save format. Documented deviations: entities are dropped, portals in clones are inert, game rules and border are not carried over. |
| IV. Foundation Rigor | Pass | The foundation is complete for its declared scope: templates, copy-on-write, creation targets, removal, reconnect, saved-data safety. Out-of-scope items (see A8) are listed, not stubbed. |
| V. Explicit Failure | Pass | Every binding returns `ApiResult`; template load, clone, create and remove report failures with a message. Nothing is skipped silently. |
| VI. Concurrency & Dependencies | Pass | Loading uses existing tokio and rayon pools; creation and removal publish through the existing job stage. No new threads or crates. |
| VII. Verified Behavior | Pass | Tests cover reader safety (no writes), copy-on-write, independent clocks, lasting-domain saves, relocation, reconnect and a full native flow ([quickstart](quickstart.md)). |

No violations; Complexity Tracking lists the one shared-host change set.

## Project Structure

### Documentation (this feature)

```text
specs/005-world-templates/
├── plan.md
├── spec.md
├── research.md
├── data-model.md
├── quickstart.md
├── tasks.md
├── contracts/
│   └── host-api.md
└── checklists/
    └── requirements.md
```

### Source Code

```text
steel-core/src/
├── chunk_saver/
│   ├── format.rs                  # EXTENDED validate_entries, RegionPos::from_filename
│   ├── region_manager.rs          # EXTENDED uses shared header validation
│   ├── region_reader.rs           # NEW strict read-only reader
│   └── ram_only.rs                # EXTENDED template base (copy-on-write)
├── chunk_saver/storage/mod.rs     # EXTENDED RamClone arm, pass the pool to RAM loads
├── config.rs                      # EXTENDED WorldStorageConfig::RamClone
├── level_data/mod.rs              # EXTENDED read_snapshot
├── loot/random_sequences.rs       # EXTENDED domain accessor
├── scoreboard.rs, command/storage.rs   # FIXED save target (B1, research R6)
├── world/
│   ├── template.rs                # NEW WorldTemplate
│   └── mod.rs                     # EXTENDED retiring flag, retire_runtime
└── server/
    ├── world_operations.rs        # NEW (moved from temporary/mod.rs)
    ├── templates.rs               # NEW registry and load
    ├── worlds.rs                  # EXTENDED DomainEntry, runtime_worlds, clock_owners
    ├── player_lifecycle.rs        # EXTENDED reconnect and respawn rules
    └── temporary/
        ├── mod.rs                 # EXTENDED request, targets, destroy waits on removals
        ├── preparation.rs         # EXTENDED template storage
        └── removal.rs             # NEW RemoveWorldJob

steel-extension-api/
├── src/runtime/types.rs, runtime/bindings.rs   # EXTENDED new types and Server methods
├── src/host/runtime.rs                         # EXTENDED adapters
└── tests/
    ├── world_templates.rs                      # NEW native test
    └── world-templates-probe/                  # NEW probe extension used by that test
```

**Structure decision.** Host changes sit where each concept already lives. The only new modules are
the reader, the template, the registry, the operations table and the removal job, each with one
responsibility. The probe extension keeps native testing inside the Steel repo.

## Platform gaps (shared host work)

These change crates every extension relies on, so they are serialized by the user
(`specs/extensions-workflow.md`). Each gap is a task in [tasks.md](tasks.md).

| # | Gap | Needed by | Tasks |
| --- | --- | --- | --- |
| G1 | Read-only region reader and `WorldTemplate` | clone, US1 | T002–T005 |
| G2 | Template registry and async load | clone, US1 | T006, T007 |
| G3 | Copy-on-write RAM storage and `RamClone` | clone, US1 | T008 |
| G4 | Domain map: optional default world, runtime worlds, clock owners | US1–US3 | T009 |
| G5 | Saved-data target fix for scoreboard and command storage | US2 (data safety) | T010 |
| G6 | Generalized creation, lasting-domain admission, independent clocks | US1, US2 | T011, T013 |
| G7 | Single-world removal with relocation | US3 | T016, T017 |
| G8 | Reconnect and respawn rules, inert portals | US2, US3 | T013, T014 |
| G9 | Extension API bindings and types | all | T012, T015, T018 |

**Not stubbed, out of scope** (spec A8): disk-backed runtime worlds, capturing templates from running
worlds, entity cloning, weather and border carry-over, clone caps.

## Consumers

- The `world-cloner` extension specifies its own service, configuration and commands on top
  of [contracts/host-api.md](contracts/host-api.md). That contract is the interface other extensions
  are planned against.
- Extension implementations that need these bindings wait for phases 2–5 of [tasks.md](tasks.md). Their
  own specs and plans can proceed now.

## Complexity Tracking

| Item | Why |
| --- | --- |
| Host change set G1–G9 across `steel-core` and `steel-extension-api` | The extension API has no storage reads or chunk writes, so a template cannot be built from outside. The set follows the one intended extension point in the host (`ram_only.rs:17-18`) and matches the spec 003 precedent of a platform phase. |
