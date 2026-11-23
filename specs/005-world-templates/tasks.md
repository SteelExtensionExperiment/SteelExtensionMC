---
description: "Task list for the world templates platform (read-only templates, copy-on-write RAM worlds, runtime world lifecycle, extension API bindings)"
---

# Tasks: Runtime RAM Worlds from Saved Templates (host platform)

**Input**: Design documents from `specs/005-world-templates/`

**Prerequisites**: [plan.md](plan.md), [spec.md](spec.md), [research.md](research.md),
[data-model.md](data-model.md), [contracts/](contracts/), [quickstart.md](quickstart.md)

**Tests**: required (FR-028). Host tests sit beside the code; the native test is T019.

**Organization**: by user story. Platform work that several stories share is in Phase 2.

**Where code goes**: every task edits this checkout (`steel-core`, `steel-extension-api`). The
`world-cloner` extension tasks live in `extension-example/world-cloner/specs/001-world-cloner/tasks.md`. Host
changes are shared by every extension and are serialized by the user (`specs/extensions-workflow.md`).

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel (different files, no dependency on an incomplete task)
- **[Story]**: US1–US3 from spec.md

---

## Phase 1: Setup

- [ ] T001 [P] Add the probe extension `steel-extension-api/tests/world-templates-probe/` (own `[workspace]`, `cdylib`, path dependency `../..`, modeled on `tests/event-probe`) exposing one command per new binding, and the native test entry `steel-extension-api/tests/world_templates.rs`, `#[ignore]` until the probe is built. Add the probe's library path next to the `event_probe` special case in `tests/common/mod.rs:220-232`.

---

## Phase 2: Foundational (blocking prerequisites)

**Purpose**: templates, storage and the world map, shared by every story.

- [x] T002 Move header validation into `steel-core/src/chunk_saver/format.rs` as `RegionHeader::validate_entries` and `ChunkEntry::validate` (from `region_manager.rs:219-260`) and make `RegionManager::open_region` use them. Add `RegionPos::from_filename` next to `filename()` (`format.rs:1219`). Test: file name round trip including negative coordinates; the existing region manager tests stay green.
- [x] T003 Add `steel-core/src/chunk_saver/region_reader.rs` (data-model `RegionReader`). Register the module in `chunk_saver/mod.rs`. Tests: a wrong version or magic is an error and leaves the file's bytes and modification time unchanged; a corrupt chunk fails the read, names region and chunk, and leaves the file unchanged; entities are removed while block entities, blocks and biomes are kept; a missing region file is not created.
- [x] T004 [P] Add `LevelData::read_snapshot(path)` in `steel-core/src/level_data/mod.rs` (data-model). Test: reading writes no file and returns the saved generation settings, seed, spawn and difficulty.
- [x] T005 Add `steel-core/src/world/template.rs` with `WorldTemplate` (data-model), built from the reader output and `read_snapshot`, validating the generator with `world_generators.validate_config` and comparing generation settings as `validate_generation_settings` does (`level_data/mod.rs:324`). Tests: missing generation settings, an unregistered generator, and a dimension mismatch each fail with a clear message; a folder with no region files is a valid empty template.
- [x] T006 [P] Extract `WorldOperationId`, `allocate`, `finish` and `take_world_operation` from `steel-core/src/server/temporary/mod.rs:23-75,178-193` into `steel-core/src/server/world_operations.rs`; create and destroy use it. Test: the existing temporary world tests pass unchanged.
- [ ] T007 Add `steel-core/src/server/templates.rs` (`WorldTemplates`, `TemplateEntry`) and `Server::load_world_template` / `Server::world_template` (data-model), loading on the existing cleanup task tracker. Tests: a duplicate key is rejected; one bad and one good template each get their own state; loading does not block the caller.
- [ ] T008 Extend `steel-core/src/chunk_saver/ram_only.rs` with the template base (`RamOnlyStorage::from_template`), update the `ChunkStorage` match arms and pass the decode pool to RAM loads in `chunk_saver/storage/mod.rs:474-484`, and add `WorldStorageConfig::RamClone` in `steel-core/src/config.rs` and `world/mod.rs:398-405`. Tests: the overlay wins over the template; an untouched chunk is never copied into the overlay; an edit in one clone does not appear in another or in the template; chunk status is taken from the template entry.
- [ ] T009 Refactor `steel-core/src/server/worlds.rs` to `DomainEntry`, `runtime_worlds` and `clock_owners`; add `publish_runtime_world`; make `advance_domain_game_times` iterate clock owners; keep `has_domain` true when a domain has no default world. Add a domain accessor to `DomainRandomSequences` (`loot/random_sequences.rs`) and hold temporary-domain sequences on `DomainEntry` (assumption A4, confirm before merging). Tests: an independent clock advances on its own; a domain without a default world does not panic; derived siblings still require a shared clock.
- [ ] T010 [P] Fix the saved-data target in `steel-core/src/scoreboard.rs:564-588` and `steel-core/src/command/storage.rs:188-212` to use `worlds.default_world(domain)` and skip domains without one. Test: a runtime world that owns its clock in a lasting domain does not consume the domain's pending scoreboard or command storage save.

**Checkpoint**: templates can be read, registered and shared; the domain map supports runtime worlds.

---

## Phase 3: User Story 1 - Clone into a new temporary domain (Priority: P1) 🎯 MVP

**Goal**: ask for a copy of a template and receive a ready, independent RAM world in a new domain.

**Independent Test**: clone a saved world with a chest twice, edit one clone, confirm the other and the
template are unchanged (spec US1).

- [ ] T011 [US1] Generalize `TemporaryWorldRequest` into the runtime world request with `source` and `target` (data-model) in `steel-core/src/server/temporary/mod.rs`, use `RamClone` storage in `preparation.rs:60-85`, add `Server::clone_world`, resolve defaults from the template plus overrides, and reject duplicate or pending keys at request time. Tests in `server/temporary/tests.rs`: the clone starts at game time 0 and reads the template's blocks and block entities and has no entities; two clones' clocks are independent; a duplicate key fails before preparation; `create_temporary_world` behavior is unchanged.
- [ ] T012 [US1] Add `TemplateState`, `TemplateStatus`, `CloneTarget`, `CloneSettings` in `steel-extension-api/src/runtime/types.rs` and `load_world_template`, `world_template_status`, `clone_world` on `Server` in `runtime/bindings.rs` with adapters in `host/runtime.rs` ([contracts/host-api.md](contracts/host-api.md)). Test: argument validation and conversions.

**Checkpoint**: US1 works at the binding level. The extension can start against these types.

---

## Phase 4: User Story 2 - Add a world to an existing domain (Priority: P1)

**Goal**: clones and generated worlds join a running lasting or temporary domain without disturbing its saved data.

**Independent Test**: clone twice into a lasting domain, restart, check scoreboard, command storage and
player data survive and no clone does (spec US2).

- [ ] T013 [US2] Accept lasting and temporary domains for `DomainTarget::Existing` in `steel-core/src/server/temporary/mod.rs` and `worlds.rs`; make portal resolution return `None` for runtime worlds (`worlds.rs:257-296`); add `Server::create_world` for generated sources. Tests: a clone joins a lasting domain with its own clock and uses the domain's random sequences; a missing or closing domain is refused; portals in and into clones do nothing; nothing is written to disk for a clone.
- [ ] T014 [US2] Reconnect and respawn rules in `steel-core/src/server/player_lifecycle.rs`: `resolve_saved_world` ignores runtime worlds of lasting domains (`:212-245`), respawn falls back to the server default world for a temporary domain with no default (`:525-533`). Tests: a saved clone key, including a reused one, puts the player at the domain default with inventory kept; respawn with no default lands in the server default world.
- [ ] T015 [P] [US2] Add the `create_world` binding and adapter (`steel-extension-api/src/runtime/bindings.rs`, `host/runtime.rs`). Test: a generated world is created in a new and in an existing domain; an unknown generator or one needing configuration fails with a message.

**Checkpoint**: US1 and US2 independent.

---

## Phase 5: User Story 3 - Remove one world and move players out (Priority: P1)

**Goal**: retire a single runtime world safely, relocating players.

**Independent Test**: put two players in a clone, remove it, both arrive in the domain default world (spec US3).

- [ ] T016 [US3] Add `World::retiring` and extract `World::retire_runtime` in `steel-core/src/world/mod.rs` (from `server/temporary/mod.rs:366-372`); refuse retiring worlds in `world_changes.rs:397-411,507-526`, `jobs/teleport.rs:261-264`, portal resolution and runtime member placement; re-check in `finish_prepared_player_join` (`player_admission.rs:248+`). Tests: each entry point refuses a retiring world.
- [ ] T017 [US3] Add `steel-core/src/server/temporary/removal.rs` with `Server::remove_world` and `RemoveWorldJob` (data-model), `DomainState::removing`, and make `DestroyDomainJob` wait on it. Tests: players move to the domain default, then to the server default; a `worlds.toml` world is refused; removing a temporary domain's first world keeps the domain, `switch_domain` fails and `select_world` works; a world still being created, one already being removed, and a shared clock owner are refused; destroying the domain waits for a removal in progress.
- [ ] T018 [US3] Add the `remove_world` and `world_is_removable` bindings and adapters. Test: conversions and refusal of `worlds.toml` worlds.

**Checkpoint**: the full world lifecycle is available to extensions.

---

## Phase 6: Polish & cross-cutting

- [ ] T019 Complete the native test `steel-extension-api/tests/world_templates.rs` against the probe: build the fixture world in the test (`World::new_with_config` with `WorldStorageConfig::Disk`, `world/mod.rs:360`; set blocks and a chest, add a pig, save), hash the folder, load one good and one missing template, assert one Ready and one Failed; clone into a new temporary domain and into a lasting domain; blocks and chest are present and the pig is not; two clones diverge when one is edited; a caller-held `Operation` is polled to completion; a player is relocated when its world is removed; and the fixture hash is unchanged at the end (SC-004).
- [ ] T020 [P] Run `cargo clippy -r --workspace --all-targets --all-features`, `cargo fmt --all --check`, `typos` on the touched crates, measure SC-001 to SC-003 and replace the proposed targets with the measured ones, and update `specs/extensions-workflow.md` status.

---

## Dependencies & execution order

- **Phase 1** has one task and no dependencies.
- **Phase 2**: T002 → T003 → T005; T004 [P] joins T005; T006 [P] → T007 (needs T005); T008 needs T005; T009 and T010 [P] are independent of the reader chain.
- **US1**: T011 needs T007, T008, T009; T012 needs T011.
- **US2**: T013 needs T011 and T009; T014 needs T013; T015 needs T013 and T012.
- **US3**: T016 needs T009; T017 needs T016 and T011; T018 needs T017.
- **Polish**: T019 needs everything; T020 [P] follows.

## Parallel opportunities

- Phase 2: T004, T006, T009, T010 alongside the T002 → T003 → T005 reader chain.
- The extension can start against the binding types as soon as T012 is fixed, using the
  contract in [contracts/host-api.md](contracts/host-api.md); its create and remove service methods
  wait for T015 and T018.

## Implementation strategy

**MVP**: Phases 1–3. After that, an extension can ask for a clone into a new temporary domain. Then US2
and US3 complete the lifecycle that an arena manager needs. Land Phase 2 in small reviewable steps,
because every extension depends on it.
