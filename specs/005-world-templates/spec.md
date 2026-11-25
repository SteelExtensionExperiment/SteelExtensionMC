# Feature Specification: Runtime RAM Worlds from Saved Templates (host platform)

**Feature Branch**: `005-world-templates`

**Created**: 2026-11-23

**Status**: Draft

**Input**: User description: "Load worlds from disk storage one time into memory and copy them an unlimited number of times, with RAM-only storage, to spawn the same world multiple times for a minigame server. Instead of a `worlds.toml` file, an extension creates worlds at runtime, including in-memory worlds cloned from a world on disk. The arena manager uses it when an arena is full: it spawns a new domain with the new world, or adds the world to an existing domain. The result is a ready, loaded world in the specified domain with the specified name."

This spec is the **host platform** half. The extension that uses it (configuration, startup loading,
service `world-cloner:clone/v1`, commands) is specified in `extension-example/world-cloner/specs/001-world-cloner/`. Both halves came out of the same set of decisions; the numbers below are
shared with that spec, and the ones it owns are marked there.

Decisions taken before writing (asked of the user, rounds 1–7):

| # | Decision |
| --- | --- |
| D2 | **A template is the whole saved world.** Chunks never saved come from the generator recorded in the template's `level.toml` (in practice `steel:empty`, so air). |
| D3 | **Clones live only in RAM**, with no cap on their number, and share the template copy-on-write. |
| D4 | **Contents:** blocks, biomes, light and block entities are kept; **entities are dropped**. Template files are never modified. |
| D5 | **Targets:** a new temporary domain, or an existing domain, temporary or lasting. The caller chooses the `domain:world` key. |
| D6 | **Each clone has its own clock** (game time and day time) starting at 0 and takes nothing from sibling worlds. |
| D7 | **Defaults:** spawn, difficulty and seed come from the template, and the caller can override them and the game mode. Game rules, world border and weather are not carried over. (The extension supplies the template's default game mode from its configuration.) |
| D8 | **Loot random sequences** behave as for a `worlds.toml` world: shared per domain. |
| D9 | **Any runtime RAM world can be removed on its own**, including a temporary domain's first world. The server rejects new entries at once and moves players out to the domain's default world, else the server default world. `worlds.toml` worlds are never removable. An emptied temporary domain stays registered until destroyed. |
| D10 | **Reconnect:** a player saved in a clone of a lasting domain lands at the domain's default spawn with inventory kept. Respawn, beds and ender pearls keep vanilla handling, plus a server-default fallback when a temporary domain has no default world. |
| D11 | **Portals** in or into clones do nothing. |
| D12 | **A template that fails to load** is marked Failed and does not stop the others. |

Decisions D1 (templates declared in extension configuration and loaded at startup) and D13 (the
service contract) belong to the extension spec.

## User Scenarios & Testing *(mandatory)*

The "user" here is an extension author calling the host bindings in
[contracts/host-api.md](contracts/host-api.md), and the players that author's extension moves around.

### User Story 1 - Clone a saved world into a new temporary domain (Priority: P1)

An extension asks for a copy of the loaded template `skywars_a` as the world `arena_7:map` in the new
domain `arena_7`. A moment later the world is loaded and ready: the blocks, containers and biomes of
the saved map are all there, no animals or armor stands are, and the world's day and night clock reads 0.

**Why this priority**: this is the core of the feature. Spleef already creates temporary worlds, but
from a generator; this lets a hand-built map be stamped out repeatedly.

**Independent Test**: save a small world with a chest to a folder, load it as a template, and clone it
twice into two domains. Both clones show the same blocks and chest contents. Breaking a block in one
leaves the other, and the template, unchanged.

**Acceptance Scenarios**:

1. **Given** a Ready template, **When** a clone is requested for a new temporary domain, **Then** the request completes with the world's key and the world is loaded, at the template's spawn unless overridden.
2. **Given** a template whose saved world contains a chest and a pig, **When** it is cloned, **Then** the chest and its contents are present and the pig is not.
3. **Given** two clones of one template, **When** a player edits blocks in the first, **Then** the second and any later clone still match the template.
4. **Given** a template folder, **When** all clones are created, edited and removed, **Then** no file in the template folder was created, renamed, changed or deleted.
5. **Given** a clone request with a seed, spawn, game mode or difficulty override, **When** the world is created, **Then** the world uses the override and every unset value falls back to the template, or for game mode to the server default for new worlds.
6. **Given** two clones in the same domain or in different domains, **When** time passes, **Then** each world's game time and day time advance independently from 0.
7. **Given** a key that is already used by a loaded or pending world, **When** a clone is requested for it, **Then** the request fails with a message naming the key, before any world is prepared.

---

### User Story 2 - Add a world to an existing domain (Priority: P1)

An extension sees that every arena world in the domain `minigames` is full. It asks for another clone
as `minigames:arena_12`, or for a plain generated world, and the world joins the domain that is already
running. The domain may be a lasting one from `worlds.toml` or a temporary one.

**Why this priority**: it is the stated use of the feature. Without it an arena manager can only grow
by creating a whole new domain per arena.

**Independent Test**: with a lasting domain `minigames` configured, clone a template into
`minigames:arena_1` and `minigames:arena_2`. Players can switch into both, and after a server restart
the domain's saved scoreboard, command storage and player data are still intact while the arena worlds
are gone.

**Acceptance Scenarios**:

1. **Given** a running lasting domain, **When** a clone is requested into it, **Then** the world joins the domain with its own clock and the domain's saved data is unaffected.
2. **Given** a running temporary domain, **When** a clone is requested into it, **Then** the world joins that domain.
3. **Given** a lasting domain with clones, **When** the server restarts, **Then** the clones are not restored and nothing about them is written to disk.
4. **Given** a domain name that no loaded domain uses, **When** an existing-domain clone is requested, **Then** the request fails and no world is created.
5. **Given** a request for a plain generated world (a generator with its default configuration, no template), **When** it is created in a new or existing domain, **Then** it is a RAM world with the same lifecycle as a clone.
6. **Given** a lasting domain, **When** several clones are added and removed while players are online, **Then** the domain's scoreboard and command storage keep saving as before.

---

### User Story 3 - Remove one world and move players out (Priority: P1)

An extension is done with the arena `minigames:arena_7`. It asks to remove that single world. New
players can no longer enter it, the players still inside are moved to the domain's default world (or to
the server's default world when that is not possible), and the world is then gone.

**Why this priority**: runtime worlds that cannot be retired leak memory for the life of the process.
The host today can only retire a whole temporary domain.

**Independent Test**: put two players in a clone, remove it, and check that both end up in the domain's
default world, that the world no longer exists, and that other worlds in the domain keep running.

**Acceptance Scenarios**:

1. **Given** a clone with players, **When** its removal is requested, **Then** no player can enter it from that moment, the players are moved out, and the operation completes once the world is retired.
2. **Given** a clone in a domain whose default world exists and is not the removed world, **When** it is removed, **Then** its players arrive in that default world.
3. **Given** the removed world is the domain's default world, or the domain has no default world, **When** it is removed, **Then** its players arrive in the server default domain's default world.
4. **Given** a world from `worlds.toml`, **When** its removal is requested, **Then** the request fails and nothing changes.
5. **Given** the first world of a temporary domain, **When** it is removed while other clones remain, **Then** the domain stays registered, the remaining worlds keep running, and switching into the domain fails until it is destroyed (selecting one of its worlds still works).
6. **Given** the last world of a temporary domain, **When** it is removed, **Then** the empty domain stays registered until the domain is destroyed through the existing operation.
7. **Given** a world that is still being created, **When** its removal is requested, **Then** the request fails with "world is not loaded".
8. **Given** a world whose clock is shared by older derived temporary worlds, **When** its removal is requested, **Then** the request fails and nothing changes.

---

### Edge Cases

- A template folder written by an older or newer Steel (`FORMAT_VERSION` differs): Failed, never renamed or rewritten. Today's `RegionManager` would rename the file; the template reader must not.
- A chunk that cannot be decoded: the whole template is Failed with the region and chunk named. Today's loader deletes the chunk slot; the template reader must not.
- A template whose `level.toml` lacks generation settings, or names a generator that is not registered (for example one from an extension that is not loaded): Failed.
- The template folder contains no region files: a valid template, and every chunk comes from its generator.
- A clone requested while its template is still Loading: fails, and the caller may retry.
- A player disconnects inside a clone in a lasting domain and reconnects after the clone was removed, or after a new clone reused the key: they arrive at the domain's default world, with inventory kept.
- Death respawn in a temporary domain with no default world: falls back to the server default world.
- A world is removed while a player is changing worlds into it, or a domain switch into it is pending: the removal waits, and the player is moved out afterwards.
- Players that are bots in a removed world: handled like any other player and relocated.
- The server stops with clones still loaded: nothing is written for them.
- The same template is cloned very many times: memory grows only by each clone's own modified chunks, and every clone sees the template's chunks without copying them.

## Requirements *(mandatory)*

### Functional Requirements

**Templates**

- **FR-001**: The host MUST load a template from a Steel world folder into memory on request, without delaying any game tick, and report its state (Loading, Ready or Failed with a message).
- **FR-002**: A template MUST contain every chunk saved in its source folder. Chunks that were never saved MUST come from the generator and configuration recorded in the template's `level.toml`.
- **FR-003**: Loading MUST be strictly read-only. It MUST NOT create, rename, truncate or delete any file in the source folder, and MUST NOT write the folder's `level.toml`.
- **FR-004**: A template that fails to load MUST be recorded as Failed with a message, and MUST NOT affect other templates.
- **FR-005**: Loading MUST drop all entities from the template, and MUST keep blocks, biomes, light and block entities.
- **FR-006**: A template MUST be immutable after loading and shared by all its clones.

**Clones and creation**

- **FR-007**: A clone MUST store only the chunks it saves itself; chunks it never saves MUST be read from the shared template. The number of clones MUST NOT be limited by the host.
- **FR-008**: A clone request MUST support a new temporary domain and an existing domain, temporary or lasting, with the key chosen by the caller.
- **FR-009**: A clone request MUST fail before any preparation when the key is already used by a loaded or pending world, the target domain does not exist (existing-domain target), or the template is not Ready.
- **FR-010**: Each clone MUST have its own game time and day time starting at 0, independent of every other world.
- **FR-011**: A clone's seed, spawn and difficulty MUST default to the template's, and the caller MAY override those and the game mode per request.
- **FR-012**: A clone MUST use the loot random sequences of its domain, as worlds from `worlds.toml` do.
- **FR-013**: The host MUST also create plain generated RAM worlds (a registered generator with its default configuration, no template) in a new or existing domain, with the same lifecycle as a clone.
- **FR-014**: Creation MUST complete asynchronously through a host `Operation`, publish the world only between world ticks, and return the loaded world's key.
- **FR-015**: Runtime worlds MUST never be written to disk, and a lasting domain's saved scoreboard, command storage, random sequences and player data MUST keep saving exactly as before whether or not it contains runtime worlds.

**Removal**

- **FR-016**: The host MUST support removing a single runtime RAM world, including a temporary domain's first world, and MUST refuse worlds loaded from `worlds.toml`.
- **FR-017**: From the moment removal starts, the world MUST reject every way of entering it: world selection, domain switch, teleport jobs, portals, and joins.
- **FR-018**: The host MUST move every player (including bots) out of the world, first to the domain's default world and, if that does not exist or is the removed world, to the server default domain's default world. The operation MUST complete only when the world has no players and no pending changes targeting it.
- **FR-019**: Removing a temporary domain's first or last world MUST keep the domain registered until `destroy_temporary_domain`. A temporary domain with no default world MUST refuse `switch_domain` and MUST still allow selecting its remaining worlds.
- **FR-020**: Removal MUST fail, changing nothing, when the world is still being created, is already being removed, belongs to a closing domain, or owns a clock still used by older derived temporary worlds.
- **FR-021**: Removing a world MUST release its storage, its entity directory entries and its portal links, and fire the world unload event.
- **FR-022**: Destroying a temporary domain MUST wait for removals in progress in that domain.

**Players**

- **FR-023**: A player saved inside a runtime world of a lasting domain MUST NOT be restored into any runtime world on reconnect or switch; they MUST arrive at the domain's default world spawn with their inventory kept.
- **FR-024**: Death respawn in a temporary domain with no default world MUST fall back to the server default world. Other respawn, bed and ender-pearl handling stays as in the host today.
- **FR-025**: Portals in a runtime world, and portals targeting one, MUST do nothing.

**Quality**

- **FR-026**: Without a caller of the new bindings, the host MUST behave as it does today. Existing `create_temporary_world` and `destroy_temporary_domain` semantics, including the derived clock of siblings, MUST be unchanged.
- **FR-027**: No game tick may wait for template loading, chunk decoding, or removal. All of it MUST run on the existing background pools or the server job stage.
- **FR-028**: The host work MUST have the unit and native tests listed in [tasks.md](tasks.md), including that template files stay byte-identical.

### Key Entities

- **Template**: a named, immutable, in-memory copy of a saved world (chunks, generation settings, spawn, seed, difficulty) with a state of Loading, Ready or Failed.
- **Clone**: a RAM-only world whose chunks are a copy-on-write view of a template, with its own clock and settings.
- **Runtime world**: any world created while the server runs and never saved, clone or generated; the only kind that can be removed.
- **Domain default world**: the world players are placed in when switching into a domain; always present for `worlds.toml` domains, absent after removal in a temporary domain.
- **Operation**: the host's pollable handle for asynchronous world work; completed exactly once, owned by whoever started it.

## Success Criteria *(mandatory)*

### Measurable Outcomes

The numeric targets in SC-001 to SC-003 are proposals, not measured values. Confirm or replace them
before implementation, and measure them in the Polish phase.

- **SC-001**: A clone of a template with 400 saved chunks is ready to receive players in under 2 seconds, and the server tick is never delayed beyond its normal budget while this happens.
- **SC-002**: 100 clones of one template that were only visited, never edited, use less than 10 % more memory than 1 clone plus the template.
- **SC-003**: Removing a world with 8 players in it moves all 8 out, and the operation completes within 5 seconds of the last player leaving.
- **SC-004**: A template folder's bytes and modification times are identical after loading, 100 clone and remove cycles, and a server restart.
- **SC-005**: Across a lasting-domain restart that had clones added and removed during the previous run, scoreboard, command storage, random sequences and player data match their last saved state, and no file for any clone exists.
- **SC-006**: Without any caller of the new bindings, host behavior and test results of existing temporary-world features (Spleef) are unchanged.

## Assumptions

- **A1** (confirmed in design): a temporary domain whose first or default world was removed has no default world. `switch_domain` into it fails, `select_world` into its remaining worlds works.
- **A2** (confirmed in design): the host also creates plain generated RAM worlds. Disk-backed runtime worlds are out of scope.
- **A3** (confirmed in design): older `create_temporary_world` siblings keep their derived clock, so removing a world whose clock others still share is rejected.
- **A4** (**rejected by the user, 2026-11-24**): the domain does not hold loot random sequences for temporary domains; they stay on the domain's default world, as today. Consequence: once a temporary domain has lost its first world, `ExistingDomain` into it fails with a clear message, because a new clone would have no sequences to share. The domain itself stays registered and `select_world` into its remaining worlds works.
- **A5**: templates are written by the same Steel version that loads them. The format has no migrations at this stage of the project.
- **A6**: the default game mode is not part of `level.toml`. Callers pass it per request; the world-cloner extension fills it from its per-template configuration.
- **A7**: these host changes are shared by every extension. They are listed as platform gaps in [plan.md](plan.md), and the user decides who builds them and in what order.
- **A8**: out of scope for this iteration: persistent on-disk copies of worlds, capturing a template from a running world, entity cloning, per-position biomes beyond what the template stores, weather and world border carry-over, and a limit on clone count.
