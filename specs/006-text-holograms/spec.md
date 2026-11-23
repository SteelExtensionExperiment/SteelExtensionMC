# Feature Specification: Extension-controlled Text Holograms (host platform)

**Feature Branch**: `006-text-holograms`

**Created**: 2026-11-23

**Status**: Draft

**Input**: Needed by the `stats-visualizer` extension (`extension-example/stats-visualizer/specs/001-stats-visualizer/`):
a scoreboard that floats over blocks and whose text an extension can change at any time. Extensions have
no way to set or update display text today.

This spec is the **host platform** half. The extension that needs it is specified in
`extension-example/stats-visualizer/`. Number 004 is taken upstream and 005 is `005-world-templates`, so this is
006.

## Decisions

Proposals from the stats-visualizer planning round. H3, H5 confirmed and H6 dropped by the user on 2026-11-23.

| # | Decision |
| --- | --- |
| H1 | A **hologram** is a vanilla text display entity owned by the host and controlled by an extension through an opaque handle. The extension never sees entity ids or UUIDs. |
| H2 | Holograms are **never written to chunk data**. An extension recreates its own holograms at startup. A server restart never leaves a duplicate or a stray one. |
| H3 | The host **keeps a hologram alive across chunk unload and load**: it removes the entity with its chunk and recreates it when the chunk loads again. The extension does nothing for this. **Confirmed**: the entity is non-persistent, dropped with its chunk (like `BlockSeat`) and recreated from the record on chunk load (research R4). |
| H4 | A hologram whose world is **unloaded or removed** is dropped; its handle becomes invalid and operations on it fail with a message. The extension recreates it on the next `on_world_load`. |
| H5 | The extension sets **text** (plain or JSON component, as every text API) and a **billboard mode**; a floating board uses "face the viewer". All other display properties use vanilla defaults. **Confirmed.** |
| H6 | **Dropped** (user, 2026-11-23): `Server<'_>` carries no caller identity and `Operation` handles are unowned. A handle is an opaque `u64`, never reused in a process; any extension holding it may use it. |

## User Scenarios & Testing *(mandatory)*

The "user" is an extension author calling the host bindings in
[contracts/host-api.md](contracts/host-api.md), and the players who see the result.

### User Story 1 - Create a floating text (Priority: P1)

An extension asks for a hologram at a position in a world with a given text. Players near it see the text
floating there, turned toward them.

**Why this priority**: the base capability; the stats board needs nothing else to display.

**Independent Test**: create a hologram with two lines in a loaded world and check that a client in range
receives a text display entity with that text and the face-the-viewer mode.

**Acceptance Scenarios**:

1. **Given** a loaded world and chunk, **When** a hologram is created, **Then** a handle is returned and nearby players see the text.
2. **Given** a world key that is not loaded, **When** a hologram is created, **Then** the call fails with a message naming the world.
3. **Given** text that starts like a JSON component but is malformed, **When** a hologram is created, **Then** it is shown as plain text, as in every other extension text API (FR-007); `parse_text` never fails.
4. **Given** a position outside the dimension's build height, **When** a hologram is created, **Then** the call fails.

---

### User Story 2 - Change the text (Priority: P1)

The extension updates the text of an existing hologram. Players who see it get the new text without the
entity being removed and recreated.

**Independent Test**: update the text and check that the client receives a metadata update for the same
entity.

**Acceptance Scenarios**:

1. **Given** a hologram, **When** its text is set, **Then** the same entity shows the new text for current and new viewers.
2. **Given** an invalid or removed handle, **When** the text is set, **Then** the call fails with a message.
3. **Given** a hologram in an unloaded chunk, **When** its text is set, **Then** the stored text changes and the entity shows it once its chunk loads.

---

### User Story 3 - Holograms survive chunk unload and restart without duplicates (Priority: P1)

Players move away and come back, and the server restarts. The hologram is there when the chunk is loaded,
exactly once, and no hologram data is left in the saved chunks.

**Independent Test**: create a hologram, unload its chunk, load it again, save the world, restart: no
hologram entity in the region files; after the extension recreates it, exactly one exists.

**Acceptance Scenarios**:

1. **Given** a hologram, **When** its chunk unloads and loads again, **Then** exactly one text display with the current text exists after the load.
2. **Given** a hologram, **When** the world saves, **Then** no hologram appears in the saved chunk or entity data.
3. **Given** a server restart, **When** the extension has not recreated the hologram yet, **Then** none exists; after it recreates it, exactly one exists.

---

### User Story 4 - Remove a hologram and drop it with its world (Priority: P2)

The extension removes a hologram, or its world goes away. No entity is left behind.

**Acceptance Scenarios**:

1. **Given** a hologram, **When** it is removed, **Then** the entity is removed for viewers and the handle is invalid.
2. **Given** a hologram in a world that is unloaded or removed, **When** that happens, **Then** its entity and record are dropped and its handle fails with a message.
3. **Given** removal of an already removed handle, **When** it is requested, **Then** it returns that nothing existed (no error).

---

### Edge Cases

- Creating a hologram in a chunk that is not loaded: the record is stored and the entity appears when the
  chunk loads (H3); no chunk is forced to load.
- Many holograms in one chunk: all recreated on load; no per-tick cost beyond what a display entity costs
  anyway.
- The extension never removes its holograms and exits: they are removed with the process; nothing remains
  on disk (H2).
- A player disconnects and reconnects: the normal entity tracker re-sends the hologram.
- A hologram in a world cloned from a template (spec 005): created like any other world; dropped when the
  runtime world is removed (H4).
- Two extensions use holograms: handles are opaque and never reused, so they do not collide.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The host MUST provide bindings to create a hologram (world key, position, text, billboard mode), set its text and remove it, each returning a message on failure and never blocking the tick.
- **FR-002**: A hologram MUST be a vanilla text display entity, with vanilla defaults for every property the binding does not set.
- **FR-003**: A hologram MUST NOT be written to chunk, entity or player data.
- **FR-004**: The host MUST recreate a hologram when its chunk loads and remove its entity when its chunk unloads, preserving the current text, with exactly one entity per hologram at any time.
- **FR-005**: The host MUST drop a hologram when its world is unloaded or removed and MUST fail later calls on its handle with a clear message.
- **FR-007**: Text MUST be accepted in the forms every other extension text API accepts (`steel-extension-api/src/text.rs`).
- **FR-008**: Existing behavior of `World::spawn_entity` and of vanilla `/summon text_display` MUST NOT change.
- **FR-009**: The change MUST have host tests beside the code and a native test with a probe extension.

### Key Entities

See [data-model.md](data-model.md): `Hologram`, `HologramId`, `HologramRegistry`.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: After a world save, a search of the saved region data finds no hologram entity.
- **SC-002**: After any number of chunk unload and load cycles, exactly one text display exists per hologram.
- **SC-003**: Setting text sends one entity-data update, not a remove and spawn.

## Assumptions

- **A1**: vanilla `Display.TextDisplay` behavior and text component rules are mirrored by the existing
  `TextDisplayEntity` (`steel-core/src/entity/entities/objects/technical/display/text_display.rs`).
  **Verified** against vanilla 26.3 source and extracted data on 2026-11-23 (research R1).
- **A2**: the entity tracker already sends display entity data to clients; no new packet is needed.
- **A3**: out of scope: item and block displays, transformation animations, interaction (click) handling,
  per-player visibility, holograms that follow an entity, and any persistence of holograms.
- **O1 (closed)**: vanilla has no server-side text length or line limit (research R1).
