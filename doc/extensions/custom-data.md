# Custom data on players and entities

Players, vanilla mobs, registered mobs, and other loaded entities use the same
typed custom-data store. The SDK exposes it as `steel_extension_api::data::Store`;
values use `data::Value`. Native serialization and the SDK access the same
compound, so there is no separate player-only copy to synchronize.

## Open a store

```rust
use steel_extension_api::{ApiResult, data::{Store, Value}};
use steel_extension_api::stabby::{abi::IDeterminantProvider, string::String as ApiString};

fn api<T: IDeterminantProvider<ApiString>>(value: ApiResult<T>) -> Result<T, String> {
    value.match_owned(Ok, Err).map_err(|error| error.to_string())
}
```

Inside a server callback, use a current `PlayerId`:

```rust
let data = api(server.player_data(player))?;
```

Inside a world callback, use an `entities::EntityId`. An event's `EntityInfo`
provides one through `identity()`:

```rust
let data = api(world.entity_data(target.identity()))?;
```

This also accepts player identities and opens the same store as `player_data`.
In a block, fluid, or registered-mob behavior callback, the borrowed
`block_behavior::types::Entity` opens its store directly:

```rust
let data = api(mob.data())?;
```

Handles do not keep entities or worlds loaded. Each operation checks the target
again. Reacquire after respawn, unloading/reloading, replacement conversion, or
restoring a stored entity: stale identities return an error. Native state
transfers can preserve data while the runtime identity changes. Run operations
on the appropriate server/world tick thread, like other gameplay APIs.
Server-opened player handles follow that incarnation's active gameplay domain;
world-opened handles require the entity to remain in that world.

## Write and share fields

Paths consist of a namespaced compound root and slash-separated fields, such as
`example:state/energy/current`. `minecraft` and unnamespaced roots are rejected
by the extension API. Root replacements must be compounds. Missing compound
parents are created; an existing scalar parent produces an error.

```rust
let mut gauge = Value::compound();
api(gauge.put("current".into(), Value::integer(20)))?;
api(gauge.put("maximum".into(), Value::integer(100)))?;
api(data.initialize("example:state/energy".into(), gauge))?;

api(data.set("example:state/label".into(), Value::text("Companion".into())))?;
```

`initialize` atomically inserts only when absent. `set` replaces exactly one
field. Neither replaces siblings, other namespaces, or unrelated native custom
data. Namespaces organize public extension state; they are not access controls.
A dependency can read and update the same documented paths.

```rust
let value: Option<Value> = api(data.get("example:state/energy/current".into()))?.into();
if let Some(value) = value {
    let current: Option<f64> = value.as_number().into();
}

let spent = api(data.spend("example:state/energy/current".into(), 5.0))?;
let current = api(data.add_clamped("example:state/energy/current".into(), 2.0, 0.0, 100.0))?;
let removed = api(data.remove("example:state/label".into()))?;
```

`get` returns an owned snapshot. Editing that `Value` does not change the entity;
write the selected field back with `set`. Prefer `spend` and `add_clamped` for
numeric updates: the read and write share one lock, and insufficient funds leave
the value unchanged. Numeric changes promote NBT numbers to doubles. Use
`Value::integer` and `as_integer` when exact signed integer records are needed.
Other value constructors include `number`, `boolean`, `bytes`, and `records`.

## Persistence and lifecycle

- Players save their entire custom compound in their gameplay-domain player
  file. Respawn copies it to the replacement player; domain switches restore the
  selected domain's saved compound. First visits start with an empty compound.
- Other serializable entities save custom data in their native entity snapshot.
  SDK mutations mark the loaded entity's chunk dirty. Chunk save/load, native
  replacement conversions that copy custom data, and the entity-storage API
  retain the compound.
- Nonpersistent entities, including custom callback projectiles, have runtime
  custom data for their lifetime; storing fields does not make them persistent.

Vanilla custom data is the entity's `data` NBT compound. Steel preserves its
ordinary field names and types, including fields outside extension namespaces.
The namespaced SDK interface edits only the requested extension fields.
Native custom-data replacement operations still replace the whole compound.

The player file keeps envelope version 10 and its existing compound-byte layout;
the shared implementation does not add a second payload or a migration.

HUD aliases and expressions read the viewing player's store. Custom data on a
different entity is not automatically a HUD binding. The entity-storage example
publishes its party's display fields in the owner's player data while the stored
entity's own custom compound travels inside its native snapshot. See
[player data and shared HUD](player-data-and-hud.md) for layout configuration.
