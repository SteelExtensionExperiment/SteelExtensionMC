# Shared HUD, player data, and example extensions

Status: both independent gameplay examples are implemented and integration-tested. Player data is per gameplay domain. Elemental spells
create world effects and temporary terrain; projectiles and spell behavior are
custom. These systems have separate modules. All HUD presentation must work
with an unmodified vanilla client and no resource pack.

Implemented foundations: shared namespaced typed entity/player compounds, atomic numeric
updates, domain-save persistence, owned sidebar/tab/boss-bar composition, data
templates and aliases, owner layout policy, and custom projectile impacts using
native thrown-item physics, chunk-persisted temporary terrain, world effects,
and owned native creature capture/deployment/combat. See [the current API](player-data-and-hud.md),
[creatures](../../extension-example/creatures/README.md), and
[magic](../../extension-example/magic/README.md).

Full scoreboard/team/tab-entry presentation remains pending. The number-format
registry extractor compiles; its server run
requires EULA acceptance in the local SteelExtractor checkout before it can
produce the data used by the remaining scoreboard work.

## Required behavior

Independent extensions must coexist without knowing about one another. Each
publishes namespaced HUD contributions and typed player data. Steel combines
them; the server owner controls layout and resolves incompatible claims. Plugin
load order and update timing must not decide which extension wins.

The creature example and magic example are separate crates. Neither depends on
the other. They share only the public HUD, player-data, entity, and item APIs.

## HUD ownership and composition

- Issue an owner-bound HUD handle through the extension lifecycle context.
  Updates and removals affect only that owner's contributions.
- Combine sidebar sections and tab header/footer sections by default, with
  deterministic ordering. Preserve rich text. Keep the server's default tab
  information as its own contribution.
- Namespace boss bars independently of caller-supplied UUIDs. Two extensions
  using the same local key must still receive separate bars.
- Support vanilla scoreboard objectives, scores, render types, display slots,
  score number formats, teams, and tab-entry presentation. The current sidebar
  wrapper is not sufficient to claim this scope is complete.
- Let owner configuration select shared properties that cannot be combined,
  such as a tab entry's single display name or a shared display slot.
- Report conflicting claims and sidebar overflow with the involved owners and
  configuration keys. Keep the last valid presentation while the owner fixes
  the layout; do not silently truncate another extension's contribution.
- Preserve packet diffing and connection-session ownership across respawns.
  Removing one contribution must leave every other contribution intact.
- Migrate the existing sidebar, boss-bar, and tab helpers through the same
  ownership mechanism; they must not remain a way to bypass composition.

## Player data and expressions

Players and other entities expose the same [custom-data store](custom-data.md),
with namespaced extension roots alongside native fields. Player roots belong to
the gameplay domain and persist through the player save/load path. Other
serializable entities retain their own compounds in native entity snapshots.
HUD expressions read the viewing player's compound.

Expose typed reads and field updates to other extensions. Updating a field must
preserve unrelated fields and namespaces. Spending mana requires an atomic
check-and-update operation so simultaneous actions cannot spend the same mana.
Dependency access must use the public data API rather than private Rust types.

The server owner can bind friendly aliases to fully qualified data paths, for
example `player_mana` to the magic extension's mana gauge. Ambiguous aliases
require owner configuration instead of choosing the last registered provider.

Templates should support expressions such as `${player_mana}` and
`${bar(player_mana, 20)}`. A gauge carries its value and maximum, so the bar can
represent changing maximum mana without assuming a fixed range. Formatting must
preserve value types until rendering and escape data inserted into rich text.

Each extension supplies usable default bindings and presentation. Owner layout
can move the same data between sidebar, boss bars, and tab sections without
requiring changes to either extension.

## Creature capture example

- Three custom-head items: Poké Ball, Super Ball, and Hyper Ball.
- Throwing an empty ball uses real projectile movement and collision.
- Capture probability increases with ball grade and decreasing target health;
  balancing belongs to example configuration rather than vanilla registry data.
- Capture eligibility must be checked against actual living-entity and
  serialization/factory capabilities. Players are not capture targets.
- Capture and release preserve the creature's native saved state. Ownership,
  active deployments, recall, entity removal, and save/load must prevent a
  creature being duplicated or lost between its world and roster representations.
- Throwing an occupied ball deploys that creature against a target. Combat uses
  real movement/navigation and damage; it cannot rely on passive animals already
  having attack goals.
- Publish roster and active-creature health data, with a default scoreboard
  section. This extension must not import the magic extension.

## Magic example

A wand opens a native menu containing nine spells: fire projectile, ice
projectile, stone projectile, fire, earth, water, lightning, projectile
protection, and physical protection. Costs, regeneration, damage, projectile
speed and durations are configurable. Area spells use the documented two-block
query cube; the earth wall uses a fixed three-by-three shape in this example.

Mana and active protection state live in player compounds. Publish the mana
gauge and a default sidebar section with current/maximum mana and a text meter
through the shared HUD. Damage protections must inspect
the actual damage source instead of guessing from the attacker's entity type.

Fire, earth and water affect the world and create temporary terrain. Custom
projectiles and spell behavior are approved. World effects, temporary terrain,
and projectiles/spells have separate modules. Visual-only lightning is not a
damaging spell implementation.

## Verified foundations and gaps

- [Current HUD API](../../steel-extension-api/src/hud.rs) replaces the previous
  sidebar and tab override. [Core HUD](../../steel-core/src/player/hud.rs) already
  diffs packets and keeps state on the connection session.
- [Scoreboard packets](../../steel-protocol/src/packets/game/c_scoreboard.rs)
  currently cover only part of vanilla presentation. [Domain scoreboards](../../steel-core/src/scoreboard.rs)
  own command data and membership, not complete client presentation.
- [Player snapshots](../../steel-core/src/player/player_data.rs) persist domain-scoped
  extension compounds inline. Runtime player attachments synchronize before player saves.
- [Menus](../../steel-extension-api/src/menus.rs),
  [item definitions](../../steel-extension-api/src/content/mod.rs), normal damage,
  explosions, native mob navigation, and thrown-projectile physics already exist.
- [World access](../../steel-extension-api/src/worlds.rs) retains ordinary world
  borrowing and clocks. [Entity storage and control](../../steel-extension-api/src/entities.rs),
  [projectiles](../../steel-extension-api/src/projectiles.rs),
  [world effects](../../steel-extension-api/src/world_effects.rs), and
  [temporary terrain](../../steel-extension-api/src/terrain.rs) are separate domains.
  The primitive lightning visual stays visual-only; the magic example explicitly
  applies native damage and ignition for its lightning spell.

Tests exercise two independent producers, identical local resource keys,
conflict reporting, owner-selected layouts, sidebar overflow, data isolation and
persistence, atomic mana spending, actual projectile impacts, capture/release
round trips, and both examples running together.
