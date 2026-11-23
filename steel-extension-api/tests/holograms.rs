//! Text holograms created by a native extension: client projection, one metadata update per
//! change, chunk unload and load, persistence, removal and world removal.
#![cfg(feature = "host")]
#![recursion_limit = "256"]
mod common;

use common::{
    Fixture, Running, command, extension_data_root, load_extensions, on_tick, start, stop,
    teleport, until,
};
use glam::DVec3;
use std::{fs, io::Cursor, path::Path, sync::Arc, time::Duration};
use steel_core::{
    entity::{
        SharedEntity,
        entities::{
            BillboardConstraints, Display as _, DisplayView as _, ItemEntity, TextDisplayEntity,
        },
    },
    permission::PermissionSubjectIndex,
    world::World,
};
use steel_registry::{
    RegistryEntry as _,
    item_stack::ItemStack,
    packets::play::{C_ADD_ENTITY, C_REMOVE_ENTITIES, C_SET_ENTITY_DATA},
    vanilla_dimension_types, vanilla_entities, vanilla_items,
};
use steel_utils::{ChunkPos, Downcast as _, WorldAabb, codec::VarInt, serial::ReadFrom as _};
use tokio::time::sleep;
use uuid::Uuid;

const HOLOGRAM_CHUNK: ChunkPos = ChunkPos::new(0, 0);
/// Far enough from the origin chunk that nothing keeps it loaded for a player there.
const FAR_AWAY: DVec3 = DVec3::new(640.5, 65.0, 640.5);
const HOME: DVec3 = DVec3::new(8.5, 65.0, 8.5);
/// Beside the hologram, so the chunk has a persistent entity to compare against.
const CONTROL_ITEM_AT: DVec3 = DVec3::new(4.5, 66.0, 4.5);
const SETTLE: Duration = Duration::from_millis(150);
const OPERATION_POLLS: usize = 100;
const UNKNOWN_HOLOGRAM: &str = "unknown or removed hologram";

/// A text display of the origin chunk as the server holds it.
struct Display {
    id: i32,
    text: String,
    billboard: BillboardConstraints,
}

fn displays(world: &World) -> Vec<Display> {
    let chunk = WorldAabb::new(0.0, 0.0, 0.0, 16.0, 128.0, 16.0);
    world
        .get_entities_in_aabb(&chunk)
        .iter()
        .filter_map(|entity| {
            let display = entity.downcast_ref::<TextDisplayEntity>()?;
            let mut seen = None;
            display.with_view(|view| {
                seen = Some((
                    format!("{:?}", view.text_ref()),
                    view.billboard_constraints(),
                ));
            });
            let (text, billboard) = seen?;
            Some(Display {
                id: entity.id(),
                text,
                billboard,
            })
        })
        .collect()
}

/// An entity packet as the client sees it.
struct EntityPacket {
    kind: i32,
    /// Entity ids addressed by the packet.
    entities: Vec<i32>,
    /// The entity type of a spawn packet.
    spawned_type: Option<i32>,
    bytes: Vec<u8>,
}

impl EntityPacket {
    fn touches(&self, kind: i32, entity: i32) -> bool {
        self.kind == kind && self.entities.contains(&entity)
    }
}

fn packets_since(running: &Running, mark: usize) -> Result<Vec<EntityPacket>, String> {
    let read =
        |cursor: &mut Cursor<&[u8]>| VarInt::read(cursor).map(|v| v.0).map_err(|e| e.to_string());
    let mut found = Vec::new();
    for packet in running.sent_packets.lock().iter().skip(mark) {
        let mut cursor = Cursor::new(packet.encoded_data.as_slice());
        let _length = read(&mut cursor)?;
        let kind = read(&mut cursor)?;
        let (entities, spawned_type) = match kind {
            C_ADD_ENTITY => {
                let id = read(&mut cursor)?;
                let _uuid = Uuid::read(&mut cursor).map_err(|e| e.to_string())?;
                (vec![id], Some(read(&mut cursor)?))
            }
            C_SET_ENTITY_DATA => (vec![read(&mut cursor)?], None),
            C_REMOVE_ENTITIES => {
                let count = read(&mut cursor)?;
                let ids = (0..count)
                    .map(|_| read(&mut cursor))
                    .collect::<Result<_, _>>()?;
                (ids, None)
            }
            _ => continue,
        };
        found.push(EntityPacket {
            kind,
            entities,
            spawned_type,
            bytes: packet.encoded_data.as_slice().to_vec(),
        });
    }
    Ok(found)
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|part| part == needle.as_bytes())
}

/// Runs a probe action and returns the line it logged.
async fn probe(running: &Running, log: &Path, action: &str) -> Result<String, String> {
    let lines = || fs::read_to_string(log).map_err(|e| e.to_string());
    let before = lines()?.lines().count();
    command(&running.server, &running.player, &format!("holo {action}")).await?;
    until(&running.player, action, || {
        lines().is_ok_and(|text| text.lines().count() > before)
    })
    .await?;
    lines()?
        .lines()
        .nth(before)
        .map(str::to_owned)
        .ok_or_else(|| format!("probe logged nothing for {action}"))
}

/// Waits for the operation started by the last probe action.
async fn finish_operation(running: &Running, log: &Path) -> Result<(), String> {
    for _ in 0..OPERATION_POLLS {
        match probe(running, log, "operation").await?.as_str() {
            "operation:ready" => return Ok(()),
            "operation:pending" => sleep(SETTLE).await,
            other => return Err(format!("operation did not finish: {other}")),
        }
    }
    Err("operation stayed pending".into())
}

/// The entities the origin chunk's saved data would restore.
async fn saved_entities(world: &Arc<World>) -> Result<Vec<SharedEntity>, String> {
    let dimension = &vanilla_dimension_types::OVERWORLD;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .map_err(|e| e.to_string())?;
    let loaded = world
        .chunk_map
        .storage
        .load_chunk(
            HOLOGRAM_CHUNK,
            dimension.min_y,
            dimension.height,
            Arc::downgrade(world),
            &pool,
        )
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the origin chunk was not saved")?;
    Ok(loaded.pending_entities)
}

async fn assert_save_excludes_hologram(world: &Arc<World>) -> Result<(), String> {
    let saved = saved_entities(world).await?;
    assert!(
        saved
            .iter()
            .any(|e| e.downcast_ref::<ItemEntity>().is_some()),
        "the control item must be saved, or this check proves nothing"
    );
    assert!(
        !saved
            .iter()
            .any(|e| e.downcast_ref::<TextDisplayEntity>().is_some()),
        "a hologram must never be written to chunk data"
    );
    Ok(())
}

#[test]
#[ignore = "requires the hologram-probe debug build (see README)"]
#[expect(
    clippy::too_many_lines,
    reason = "one linear server session drives the hologram lifecycle in order"
)]
fn holograms_follow_chunks_stay_unsaved_and_end_with_their_handle_or_world() -> Result<(), String> {
    let data_root = extension_data_root();
    let log = data_root.join("hologram-probe/holograms.log");
    let loaded = load_extensions(&[("hologram_probe", "")], &data_root)?;
    let initialized = loaded.initialized;
    let fixture = Fixture::new()?;
    let runtime = Arc::clone(&fixture.runtime);
    runtime.block_on(async {
        let home = &fixture.world;
        let uuid = Uuid::new_v4();
        let running = start(
            home,
            (loaded.extensions, loaded.commands),
            PermissionSubjectIndex::new(),
            uuid,
            "HologramTester",
        )
        .await?;
        let (server, player) = (Arc::clone(&running.server), Arc::clone(&running.player));
        let world = Arc::clone(home);
        on_tick(&server, move || {
            world
                .spawn_item(
                    CONTROL_ITEM_AT,
                    ItemStack::with_count(&vanilla_items::DIAMOND, 1),
                )
                .map(|_| ())
                .ok_or_else(|| "control item was not spawned".to_owned())
        })
        .await?;

        // Create: a client in range sees a centered text display with the text.
        assert_eq!(probe(&running, &log, "create").await?, "create:ok");
        until(&player, "hologram spawn", || displays(home).len() == 1).await?;
        let [shown] = <[Display; 1]>::try_from(displays(home)).map_err(|_| "one display")?;
        assert!(shown.text.contains("stats"), "{}", shown.text);
        assert!(matches!(shown.billboard, BillboardConstraints::Center));
        let hologram_id = shown.id;
        let text_display =
            i32::try_from(vanilla_entities::TEXT_DISPLAY.id()).map_err(|e| e.to_string())?;
        until(&player, "client spawn packet", || {
            packets_since(&running, 0).is_ok_and(|packets| {
                packets.iter().any(|p| {
                    p.touches(C_ADD_ENTITY, hologram_id) && p.spawned_type == Some(text_display)
                })
            })
        })
        .await?;

        // SC-003: a text change is one metadata update of the same entity.
        let mark = running.sent_packets.lock().len();
        assert_eq!(probe(&running, &log, "set").await?, "set:ok");
        until(&player, "metadata update", || {
            packets_since(&running, mark).is_ok_and(|packets| {
                packets
                    .iter()
                    .any(|p| p.kind == C_SET_ENTITY_DATA && contains(&p.bytes, "updated"))
            })
        })
        .await?;
        sleep(SETTLE).await;
        let packets = packets_since(&running, mark)?;
        let updates: Vec<_> = packets
            .iter()
            .filter(|p| p.kind == C_SET_ENTITY_DATA)
            .collect();
        assert_eq!(updates.len(), 1, "exactly one metadata update");
        assert_eq!(
            updates[0].entities,
            [hologram_id],
            "the same entity is updated"
        );
        assert!(
            packets.iter().all(|p| p.kind == C_SET_ENTITY_DATA),
            "no spawn or removal accompanies a text change"
        );
        let [current] = <[Display; 1]>::try_from(displays(home)).map_err(|_| "one display")?;
        assert_eq!(
            (current.id, current.text.contains("updated")),
            (hologram_id, true)
        );

        // SC-001: saving the world while the hologram is loaded writes only the control item.
        assert!(home.save_all_chunks().await.map_err(|e| e.to_string())? > 0);
        assert_save_excludes_hologram(home).await?;

        // SC-002: the chunk unloads and loads again, leaving exactly one current hologram.
        teleport(&server, &player, FAR_AWAY).await?;
        until(&player, "origin chunk unload", || {
            home.chunk_map
                .with_full_chunk(HOLOGRAM_CHUNK, |_| ())
                .is_none()
        })
        .await?;
        assert!(displays(home).is_empty());
        assert_save_excludes_hologram(home).await?;
        teleport(&server, &player, HOME).await?;
        until(&player, "hologram respawn", || displays(home).len() == 1).await?;
        sleep(SETTLE).await;
        let [reloaded] = <[Display; 1]>::try_from(displays(home)).map_err(|_| "one display")?;
        assert!(reloaded.text.contains("updated"), "{}", reloaded.text);
        assert!(matches!(reloaded.billboard, BillboardConstraints::Center));

        // US4: remove, remove again, and a dead handle fails.
        assert_eq!(probe(&running, &log, "remove").await?, "remove:true");
        assert!(displays(home).is_empty());
        assert_eq!(probe(&running, &log, "remove").await?, "remove:false");
        assert_eq!(
            probe(&running, &log, "set").await?,
            format!("set:err:{UNKNOWN_HOLOGRAM}")
        );

        // H4: a hologram of a removed world is gone with it.
        assert_eq!(
            probe(&running, &log, "temp_world").await?,
            "temp_world:started"
        );
        finish_operation(&running, &log).await?;
        assert_eq!(
            probe(&running, &log, "temp_create").await?,
            "temp_create:ok"
        );
        assert_eq!(probe(&running, &log, "temp_set").await?, "temp_set:ok");
        assert_eq!(
            probe(&running, &log, "temp_destroy").await?,
            "temp_destroy:started"
        );
        finish_operation(&running, &log).await?;
        assert_eq!(
            probe(&running, &log, "temp_set").await?,
            format!("temp_set:err:{UNKNOWN_HOLOGRAM}")
        );

        stop(running, &initialized, home).await?;
        Ok(())
    })
}
