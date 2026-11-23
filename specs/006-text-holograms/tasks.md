---
description: "Task list for the extension hologram host platform"
---

# Tasks: Extension-controlled Text Holograms (host platform)

**Input**: Design documents from `specs/006-text-holograms/`

**Prerequisites**: [plan.md](plan.md), [spec.md](spec.md), [research.md](research.md),
[data-model.md](data-model.md), [contracts/](contracts/), [quickstart.md](quickstart.md). Decisions H3, H5, H6 are settled in spec.md.

**Tests**: required (FR-009).

**Where code goes**: every task edits this checkout. The `stats-visualizer` tasks live in
`extension-example/stats-visualizer/specs/001-stats-visualizer/tasks.md`. Host changes are shared and serialized by the user (`specs/extensions-workflow.md`).

## Format: `[ID] [P?] [Story] Description`

---

## Phase 1: Setup and verification (blocking)

- [x] T001 Generate `minecraft-src/` with `update-minecraft-src.sh` (or obtain it from the user) and verify spec assumption A1 and research R1 against vanilla `Display.TextDisplay`: defaults, billboard values, line-width and text limits, tracking and view range, and what vanilla persists. Record findings in `research.md`; stop and ask for anything that does not match `TextDisplayEntity`.
- [x] T002 Read `World::on_entity_chunk_unload_start/finalized` (`steel-core/src/world/entity_management.rs:377-400`), the entity manager and `BlockSeat` unload handling, and decide the H3 mechanism (keep entity with chunk, or remove and recreate). Record it in `research.md` R4. Stop and ask if both options are unsuitable.
- [ ] T003 [P] Add `steel-extension-api/tests/hologram-probe/` (own `[workspace]`, `cdylib`, modeled on `tests/event-probe`) with one command per binding, and the native test entry `tests/holograms.rs`, `#[ignore]` until built. Add its library path next to the `event_probe` special case in `tests/common/mod.rs`.

---

## Phase 2: Foundation

- [ ] T004 Make a text display non-persistent for holograms without changing `/summon text_display` (FR-008): the mechanism chosen after T001 and T002 (a flag on `TextDisplayEntity` or a Steel-owned wrapper overriding `Entity::is_persistent`, `entity/entity/mod.rs:172-176`). Tests: a hologram entity is absent from saved chunk data; a summoned text display still saves.
- [ ] T005 Add `steel-core/src/world/hologram.rs` (`HologramRegistry`, `Hologram`, data-model) and hook chunk load, chunk unload and world unload into it. Tests: exactly one entity after repeated unload and load cycles; text set while unloaded shows after load; world unload drops the records and entities; a removed handle fails.

**Checkpoint**: the host can hold holograms through the whole chunk lifecycle.

---

## Phase 3: User Stories 1 and 2 (Priority: P1) 🎯 MVP

- [ ] T006 [US1] Add `HologramId` and `Billboard` in `steel-extension-api/src/runtime/types.rs` and the `create_hologram` binding with adapter in `host/runtime.rs` ([contracts/host-api.md](contracts/host-api.md)); validate world, text and build height. Tests: each failure path; success returns a handle.
- [ ] T007 [US2] Add `set_hologram_text` and `remove_hologram`. Tests: one entity-data update per change; invalid handle message; remove returns `false` the second time.

---

## Phase 4: User Stories 3 and 4

- [ ] T008 [US3] Complete the native test `tests/holograms.rs` against the probe: create, update, unload and load a chunk, save the world and inspect region data for holograms (SC-001), restart-style recreation yields one entity (SC-002), update sends one metadata update (SC-003).
- [ ] T009 [US4] Native test for remove and world unload (H4): handles fail afterwards, nothing remains.

---

## Phase 5: Polish

- [ ] T010 [P] Document the bindings and the lifecycle rules in `doc/extensions/api.md`, including that extensions recreate holograms on `on_world_load`.
- [ ] T011 [P] Run `cargo clippy -r --workspace --all-targets --all-features`, `cargo fmt --all --check`, `typos`, and the manual check in [quickstart.md](quickstart.md).

---

## Dependencies & execution order

- **T001** and **T002** first; they gate T004 and T005. **T003** is parallel.
- T004 → T005 → T006 → T007. T008 needs T003, T007. T009 needs T008.
- T010, T011 follow.

## Implementation strategy

**MVP**: T001–T007. After that the stats board can create and update holograms. T008–T009 prove the
lifecycle. The `stats-visualizer` tasks T006, T008 and T009 wait on T007.
