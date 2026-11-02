# Player data and shared HUD

These APIs run entirely on the server. Vanilla clients receive ordinary
scoreboard, boss-bar and tab-list packets. They need neither client mods nor
resource packs. Templates are evaluated before packets are sent.

## Player data

Players and other entities share [one custom-data API](custom-data.md).
`Server::player_data(player)` opens a `data::Store`; `World::entity_data(id)`
opens the same store for a loaded player or another entity. Borrowed entities in
behavior callbacks expose `data()` too. All stores offer `get`, `set`,
`initialize`, `spend`, `add_clamped`, and `remove`, using typed `data::Value` values.

Paths start with a namespaced compound root, such as `magic:state/mana/current`.
Writing a field preserves siblings, other namespaces, and unrelated native custom
data. Reads return owned snapshots. Dependencies can use the same public paths;
no downcasting or private extension types are involved. Defaults and numeric
updates can be atomic through `initialize`, `spend`, and `add_clamped`.

Data belongs to the current gameplay domain. Domain changes restore that
domain's snapshot; a first visit starts empty. Death and respawn preserve it,
independently of inventory-drop rules. Player storage now uses envelope version
10, embedding the shared custom compound in the same atomic player file.
Unifying the store preserves that version and byte layout. Version 9 files are
incompatible; this early-development change provides no migration.

## HUD sources

`export_extension!` binds a `default` source during pre-init. Existing
`Server::set_sidebar`, `set_boss_bar` and `set_tab_list` helpers publish through
that source. They no longer replace another extension's contributions.

Register more sources with `Hooks::hud_source` or
`ExtensionContext::hud_source`. The host supplies the calling extension's
identity. Source names visible in owner configuration are
`extension-name:local-source`, for example `magic:default`.

Each source supports sidebar sections, tab header/footer sections, and boss
bars with all colors, segment styles and three vanilla flags. Bar UUIDs are
local to a source: identical UUIDs from two extensions produce different bars.
Clear methods remove only the selected source/resource. Updates send only
changed fields and lines, and presentation lives on the connection across
respawns.

Sections combine in lexical source order by default. The server's TPS tab
information contributes alongside extensions. An empty section title keeps its
lines and adds no title label or separator. An owner can still set `sidebar_title`.
The vanilla sidebar limit is 15 lines. Overflow reports the sources and retains
the last valid sidebar;
unrelated bar/tab changes can still succeed. Desired contributions remain
registered so owner layout changes can resolve the conflict.

## Data expressions

A gauge is a compound with numeric `current` and `maximum` fields. Register a
default friendly alias with `source.bind("player_mana", "magic:state/mana")`
(convert string arguments with `.into()` in Rust). A data-driven bar uses
`Source::set_gauge_bar`; its progress follows the gauge automatically.

Text templates accept `${player_mana}`, `${magic:state/mana/current}`, and
`${bar(player_mana, 20)}`. The gauge's plain value is its `current`; `bar`
uses `current / maximum`. Width is bounded to 1–256. Missing fields suppress
their dependent line or bar. Invalid types or bounds produce a diagnostic.

Plain text and JSON text components are supported. Substitution happens inside
JSON string values, so data cannot inject component properties. An extension's
own alias is unambiguous within its source. Owner templates must resolve aliases
with multiple providers explicitly. Dependency writes refresh presentation on
the next player tick; unchanged data avoids template work and packets.

## Owner configuration

For the `magic-example` and `creatures-example` examples in `config/config.toml`:

```toml
[server.hud]
order = ["creatures-example:default", "magic-example:default"]
sidebar_title = "Adventure"
# To replace all default sidebar sections with your own layout:
# sidebar_lines = ["Mana: ${player_mana}/${player_max_mana}", "${bar(player_mana, 20)}"]
# hidden = ["magic-example:default"]
# hidden = ["steel:server"] # suppress the default server tab information

[server.hud.aliases]
player_mana = "magic:state/mana"
player_max_mana = "magic:state/mana/maximum"

```

For a complete custom combination, merge [combined-hud.toml](examples/combined-hud.toml)
into that file. It replaces the sidebar with a colored mana meter and the names
and health of the first three party slots, and mixes creature and mana data in
the tab header. Edit the templates to reorder, interleave, or combine fields
from either extension. Empty slots suppress their rows. Extensions keep
publishing their own data and defaults; they do not need to communicate.

`order` arranges default source sections. `sidebar_lines` replaces those sections
with the owner's exact layout, so fields from different extensions can appear
on the same line. Omitting it restores the automatic combination. Alias bindings
select the data provider explicitly when names conflict.

`tab_header` and `tab_footer` replace the combined defaults with owner templates.
For extensions that publish boss bars, `boss_bars` settings use keys of the form
`extension-name:source/local-uuid` and support `title`, `gauge`, and `hidden`.
Configuration takes effect on restart. `Server::hud_conflicts` exposes current
diagnostics; changed failures are also logged once.

Full objective/team/score-number-format and individual tab-entry ownership are
still pending; the section API is not yet the complete vanilla scoreboard API.

## Custom projectiles

`World::throw_projectile` launches an `ItemSpec` from a connected player's eyes
using native thrown-projectile aiming, drag, movement and collision. The
`projectiles::ProjectileSpec` supplies speed, uncertainty, gravity and a finite
lifetime. The client sees an item-carrying vanilla snowball; a registered custom
head can supply its item appearance. Launching does not consume inventory.
Zero gravity synchronizes vanilla's `NoGravity` metadata, so the client predicts
straight spell flight. Its normal snowball prediction matches gravity `0.03`;
other positive gravity values need server corrections.
`World::throw_projectile_towards` accepts an explicit world-space hit point.
`InteractEntityEvent::hit_position` exposes that point from the interaction
packet, so an entity click can aim a projectile without waiting for a movement
packet. Both launch methods retain native movement inheritance and collision.

An `extern "C" fn(Server<'_>, projectiles::Impact<'_>, u64)` callback receives the world,
exact impact point, optional entity target and optional shooter once per impact.
`Impact::item` exposes the projectile's launch item and its attached extension
data, so callbacks can validate a retained control identity before acting.
Use `#[stabby::export(callback)]` as with other callbacks. Impacts occur on the
world tick without locks held across the extension boundary. Lifetime expiry
and unloading discard these ephemeral projectiles. Independent portal travel is disabled
because callbacks have no destination-entity save representation. They are not saved as
vanilla snowballs. Callbacks define gameplay effects explicitly.


## World effects and temporary terrain

These are separate domains from projectile movement. `world_effects::EntityId`
pairs a network ID with the persistent UUID, so delayed operations reject stale
identities. `World::entities_near` queries loaded living entities in a cube.
`entity_vitals`, `hurt_entity`, `affect_entity` and `ignite_entity` use native
health, damage, armor, effect and fire handling. Damage causes and effect keys
resolve through the actual registry. `Impact::hurt_target` retains the exact
projectile and its shooter for native attribution.

`DamageEvent::cause_in_tag` classifies the real damage cause and reports unknown
tags. For player victims, `DamageEvent::player_server` exposes a borrowed optional
server capability so protection rules can read the live player compound during
that damage event. Mob-only events have no player context.

`World::set_temporary_block(pos, expected, placed, ticks)` performs a conditional
native block write. It rejects block entities, overlapping claims and unavailable
chunks. Restoration metadata is recorded in the same palette transaction before
callbacks. Any subsequent state change cancels that claim, even a change away
and back. Expiry uses `World::game_time`, survives unload/restart with the chunk,
and resumes when the chunk is block-ticking again; it never forces chunk loads.
Native neighbor changes, fire spread and fluid flow are ordinary world behavior;
the API does not roll back an entire region or erase later player edits.

## Native entity storage and control

The SDK's `entities` module has no party, capture, fainting, or battle concept.
`World::store_entity` saves a native serializable entity at a vacant player-data
path. `Server::restore_entity` recreates it at a loaded, collision-free position.
`return_entity` snapshots and removes a live instance; `release_entity` transfers
it back to ordinary chunk persistence and removes its stored compound. Failures
retain ownership. Nonliving entities, such as dropped items, are supported too.
Passengers, vehicles, leashes, players and unsupported native factories are rejected.

`EntityPolicy::default()` preserves native mortality and death loot. An extension
can explicitly supply `minimum_health` and `drop_loot`; the minimum is applied
before native death processing, including external damage and direct health writes.
Restoring a dead snapshot with a positive minimum clears its native death state.
Release removes the policy. The creatures example chooses a minimum of one;
that value is not built into Steel or the SDK.

`synchronize_stored_entities` publishes live state into player data, preserving
extension/dependency fields. `stored_entity_instances` returns each path, current
native identity and a storage activation identity. The latter survives native
replacement conversions and player respawn, and changes on a new restoration.
Native UUIDs prevent duplicate instances. Player saves synchronize attachments;
disconnect, domain departure and shutdown return live instances to storage.
Stored instances cannot independently cross portals because their authoritative
state belongs to their player's domain compound.

`World::control_entity` installs an exclusive, cancellable AI callback. Returning
false retires it. `EntityControl` exposes native movement toward another entity,
look control, melee reach/visibility checks and correctly attributed native damage.
Sensing, navigation, movement controls, collision and physics remain native.
Direct-flight mobs use their flight capability; ground mobs use navigation.
These operations can support companions, NPCs, minigames, or other custom AI.

The [creatures example's battle module](../../extension-example/creatures/src/battle.rs)
defines reciprocal attacks, damage, timing, distance limits, the one-health minimum,
and battle completion. Its own heartbeat updates the HUD and returns participants
to their balls. Capture chances, ball grades, party size, release buttons and stale
ball checks also live in the example. Native entity conversions and bats use the
same generic APIs; neither requires a creature-specific core service.

Core only supplies runtime player/entity attachments, persistent block-state
attachments, lifecycle callbacks and native mechanics. SDK host adapters implement
entity storage, custom projectiles and temporary terrain using those capabilities.
Temporary terrain claims are written with their block palette transaction; later
state changes cancel them, including changing away and back to the same state.
State references persist through the chunk's named palette, while attachment
payloads remain opaque to core. Expiration runs in an SDK tick subscription.

## Examples and save formats

[Creatures](../../extension-example/creatures/README.md) and
[Magic](../../extension-example/magic/README.md) are separately linked libraries
with no mutual dependency. Their integration test drives actual client packet
handlers, native impacts, all nine spells and both HUD contributions together.
Neither needs a modified client or resource pack.
Their default sidebar combines mana (current/maximum and a text meter) with the
first three captured creatures (name, current/maximum health, ball grade and
deployment status). All six party slots remain available in the creature menu.

Player compounds use player format 10. Temporary terrain uses chunk region
format 25. This early-development change does not migrate older formats; keep
existing saves separate and use a fresh test world when exercising it. No saves
are deleted automatically. Rebuild native extension libraries after SDK changes.
