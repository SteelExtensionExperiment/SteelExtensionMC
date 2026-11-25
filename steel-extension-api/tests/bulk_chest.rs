//! Multiblock assembly, native inventory transfers, and chunk persistence.
#![cfg(feature = "host")]
#![recursion_limit = "256"]

mod common;

use std::{collections::BTreeSet, io::Cursor, sync::Arc, time::Duration};

use glam::Vec3;
use rustc_hash::FxHashMap;
use steel_core::{
    behavior::BlockHitResult,
    block_entity::{BLOCK_ENTITIES, StorageBlockEntity},
    chunk::test_support::insert_full_chunk,
    entity::{Entity as _, next_entity_id},
    inventory::{container::Container as _, lock::ContainerLockGuard},
    player::{
        ClientInformation, Player, PlayerConnection, PlayerSession,
        connection::{JavaConnection, OutboundPacket},
    },
};
use steel_protocol::{
    packet_traits::EncodedPacket,
    packets::game::{ClickType, HashedStack, SAcceptTeleportation, SContainerClick, SUseItemOn},
};
use steel_registry::{
    REGISTRY, RegistryEntry as _, RegistryExt as _,
    blocks::properties::Direction,
    data_components::vanilla_components::DAMAGE,
    entity_data::{EntityData, write_data_values},
    game_profile::GameProfile,
    item_stack::ItemStack,
    items::ItemRef,
    packets::play::{
        C_ADD_ENTITY, C_LEVEL_CHUNK_WITH_LIGHT, C_OPEN_SCREEN, C_PLAYER_POSITION,
        C_REMOVE_ENTITIES, C_SET_ENTITY_DATA, C_SYSTEM_CHAT,
    },
    vanilla_blocks, vanilla_entities,
    vanilla_entity_data::ItemDisplayEntityData,
    vanilla_game_rules::RESPAWN_RADIUS,
    vanilla_items,
};
use steel_utils::{
    BlockPos, BlockStateId, ChunkPos, Downcast as _,
    codec::VarInt,
    locks::AsyncMutex,
    serial::ReadFrom as _,
    types::{InteractionHand, UpdateFlags},
};
use text_components::TextComponent;
use tokio::{sync::mpsc, time::timeout};
use tokio_util::sync::CancellationToken;

const CORE: BlockPos = BlockPos::new(8, 65, 10);
const NEAREST_WALL: BlockPos = BlockPos::new(9, 65, 10);
const TOP_CENTER: BlockPos = BlockPos::new(9, 67, 11);
const EDGE: i32 = 3;
const SMALL_CORE: BlockPos = BlockPos::new(15, 65, 10);
const SMALL_EDGE: i32 = 2;
const CAPACITY: usize = 729;
const TARGET_PAGE: usize = 7;
const FIRST_SLOT_OF_TARGET_PAGE: usize = 270;
const NEXT_PAGE_BUTTON: i16 = 53;
const FIRST_HOTBAR_MENU_SLOT: i16 = 81;

#[test]
#[ignore = "requires cargo build --manifest-path extension-example/bulk-chest/Cargo.toml"]
fn bulk_chest_forms_one_display_and_preserves_the_shared_inventory() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let loaded = common::load_extensions(&[("bulk_chest", "")], dir.path())?;
    assert_eq!(loaded.order, ["bulk-chest"]);
    let fixture = common::Fixture::new()?;
    fixture.world.set_game_rule(&RESPAWN_RADIUS, 0);
    for x in -2..=2 {
        for z in -2..=2 {
            if x != 0 || z != 0 {
                let _ = insert_full_chunk(&fixture.world, ChunkPos::new(x, z), true);
            }
        }
    }
    fixture.runtime.block_on(async {
        let uuid = uuid::Uuid::new_v4();
        let running = common::start(
            &fixture.world,
            (loaded.extensions, loaded.commands),
            common::grants(uuid, &[])?,
            uuid,
            "BulkTester",
        )
        .await?;
        let result = exercise_storage(&running).await;
        common::stop(running, &loaded.initialized, &fixture.world).await?;
        result
    })
}

async fn exercise_storage(running: &common::Running) -> Result<(), String> {
    let core = block("bulk-chest:core")?;
    let wall = block("bulk-chest:wall")?;
    let core_item = item("bulk-chest:core")?;
    let wall_item = item("bulk-chest:wall")?;
    common::until(&running.player, "initial chest chunks", || {
        let sent = running.sent_packets.lock();
        [CORE, SMALL_CORE.offset(SMALL_EDGE - 1, 0, 0)]
            .into_iter()
            .all(|pos| {
                let ChunkPos(chunk) = ChunkPos::from_block_pos(pos);
                sent.iter().any(|packet| {
                    let mut payload = Cursor::new(packet.encoded_data.as_slice());
                    VarInt::read(&mut payload).is_ok()
                        && VarInt::read(&mut payload)
                            .is_ok_and(|id| id.0 == C_LEVEL_CHUNK_WITH_LIGHT)
                        && i32::read(&mut payload).is_ok_and(|x| x == chunk.x)
                        && i32::read(&mut payload).is_ok_and(|z| z == chunk.y)
                })
            })
    })
    .await?;
    let teleport = running.sent_packets.lock().iter().rev().find_map(|packet| {
        let mut payload = Cursor::new(packet.encoded_data.as_slice());
        VarInt::read(&mut payload).ok()?;
        if VarInt::read(&mut payload).ok()?.0 != C_PLAYER_POSITION {
            return None;
        }
        Some(VarInt::read(&mut payload).ok()?.0)
    });
    let player = Arc::clone(&running.player);
    let owner = common::on_tick(&running.server, move || {
        if player.is_awaiting_teleport() {
            let teleport_id = teleport.ok_or("missing initial teleport packet")?;
            let position = player.position();
            player.handle_accept_teleportation(SAcceptTeleportation {
                teleport_id,
                x: position.x,
                y: position.y,
                z: position.z,
                y_rot: 0.0,
                x_rot: 0.0,
            });
            assert!(!player.is_awaiting_teleport());
        }
        let world = player.get_world();
        player.inventory.lock().set_selected_slot(0);
        for y in 0..EDGE {
            for x in 0..EDGE {
                for z in 0..EDGE {
                    let pos = BlockPos::new(CORE.x() + x, CORE.y() + y, CORE.z() + z);
                    if pos != TOP_CENTER && (x != 1 || y != 1 || z != 1) {
                        let state = if pos == CORE { core } else { wall };
                        let item = if pos == CORE { core_item } else { wall_item };
                        place_part(&player, pos, Direction::Up, item);
                        assert_eq!(world.get_block_state(pos), state, "placing at {pos:?}");
                    }
                }
            }
        }
        use_part(&player, CORE);
        assert!(player.open_container_id().is_none());
        place_part(&player, TOP_CENTER, Direction::East, wall_item);
        assert_eq!(world.get_block_state(TOP_CENTER), wall);
        let owner = world.get_block_entity(CORE).ok_or("missing core entity")?;
        let packet = world
            .chunk_map
            .with_full_chunk(ChunkPos::from_block_pos(CORE), |chunk| {
                chunk.extract_chunk_data()
            })
            .ok_or("the core chunk is unloaded")?;
        assert!(
            packet.block_entities.is_empty(),
            "server-only storage leaked into the chunk packet"
        );
        assert!(
            player.is_within_block_interaction_range(NEAREST_WALL),
            "wall is out of reach at {:?}",
            player.position(),
        );
        use_part(&player, NEAREST_WALL);
        Ok(owner)
    })
    .await?;
    let first = current_menu(running, "Bulk Chest  1/17")?;
    let chest_field = chest_display_fields(EDGE)?;
    common::until(&running.player, "assembled vanilla chest display", || {
        chest_displays(running, &chest_field).len() == 1
    })
    .await?;
    let first_display = chest_displays(running, &chest_field)
        .pop_first()
        .ok_or("missing vanilla chest display")?;

    let player = Arc::clone(&running.player);
    common::on_tick(&running.server, move || go_to_target_page(&player, first)).await?;
    let target = current_menu(running, "Bulk Chest  7/17")?;
    let player = Arc::clone(&running.player);
    common::on_tick(&running.server, move || {
        let mut pickaxe = ItemStack::new(&vanilla_items::IRON_PICKAXE);
        pickaxe.set(DAMAGE, 23);
        player.inventory.lock().set_item(0, pickaxe);
        click(
            &player,
            target,
            FIRST_HOTBAR_MENU_SLOT,
            ClickType::QuickMove,
        );
        assert!(player.inventory.lock().get_item(0).is_empty());
        player.get_world().set_block(
            NEAREST_WALL,
            vanilla_blocks::AIR.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        Ok(())
    })
    .await?;
    common::until(&running.player, "storage close after shell break", || {
        running.player.open_container_id().is_none()
    })
    .await?;
    common::until(
        &running.player,
        "removal of the assembled chest display",
        || chest_displays(running, &chest_field).is_empty(),
    )
    .await?;

    let world = running.player.get_world();
    let reloaded = BLOCK_ENTITIES.create_and_load_owned_or_unimplemented(
        owner.get_type(),
        Arc::downgrade(&world),
        CORE,
        core,
        owner.save_with_full_metadata(),
    );
    let storage = reloaded
        .downcast_ref::<StorageBlockEntity>()
        .ok_or("saved core lost its storage type")?;
    assert!(
        storage.prepare_inventory(216).is_err(),
        "a smaller shell would hide the stored item"
    );
    let container = storage.prepare_inventory(CAPACITY)?;
    let guard = ContainerLockGuard::lock_all(&[&container]).map_err(|error| error.to_string())?;
    let inventory = guard
        .get(container.container_id())
        .ok_or("missing restored inventory")?;
    let pickaxe = inventory.get_item(FIRST_SLOT_OF_TARGET_PAGE);
    assert!(pickaxe.is(&vanilla_items::IRON_PICKAXE));
    assert_eq!(pickaxe.get(DAMAGE), Some(&23));
    drop(guard);

    let player = Arc::clone(&running.player);
    common::on_tick(&running.server, move || {
        player
            .get_world()
            .set_block(NEAREST_WALL, wall, UpdateFlags::UPDATE_ALL);
        use_part(
            &player,
            BlockPos::new(CORE.x() + EDGE - 1, CORE.y() + 1, CORE.z()),
        );
        let first = player
            .open_container_id()
            .ok_or("repaired chest did not open")?;
        let target = go_to_target_page(&player, first)?;
        click(&player, target, 0, ClickType::QuickMove);
        assert!(player.inventory.lock().get_items().iter().any(|item| {
            item.is(&vanilla_items::IRON_PICKAXE) && item.get(DAMAGE) == Some(&23)
        }));
        Ok(())
    })
    .await?;
    common::until(
        &running.player,
        "replacement chest display after repair",
        || {
            let displays = chest_displays(running, &chest_field);
            displays.len() == 1 && !displays.contains(&first_display)
        },
    )
    .await?;

    let player = Arc::clone(&running.player);
    common::on_tick(&running.server, move || {
        let world = player.get_world();
        for x in 0..SMALL_EDGE {
            for y in 0..SMALL_EDGE {
                for z in 0..SMALL_EDGE {
                    let pos = SMALL_CORE.offset(x, y, z);
                    let state = if pos == SMALL_CORE { core } else { wall };
                    world.set_block(pos, state, UpdateFlags::UPDATE_ALL);
                }
            }
        }
        Ok(())
    })
    .await?;
    let smaller_fields = chest_display_fields(SMALL_EDGE)?;
    common::until(&running.player, "second assembled chest display", || {
        chest_displays(running, &smaller_fields).len() == 1
    })
    .await?;
    assert_two_displays(&running.sent_packets.lock(), &chest_field, &smaller_fields);
    check_packed_displays(running, &chest_field, &smaller_fields).await
}

async fn check_packed_displays(
    running: &common::Running,
    larger: &ChestDisplayFields,
    smaller: &ChestDisplayFields,
) -> Result<(), String> {
    let profile = GameProfile::new(uuid::Uuid::new_v4(), "PackedBulk".into(), Vec::new())
        .map_err(|error| error.to_string())?;
    let (outgoing, mut packets) = mpsc::unbounded_channel();
    let session = Arc::new(PlayerSession::new(10, 10));
    let id = next_entity_id();
    let connection = JavaConnection::new(
        outgoing,
        CancellationToken::new(),
        None,
        Arc::new(AsyncMutex::new(None)),
        u64::try_from(id).map_err(|error| error.to_string())?,
        Arc::clone(&session),
    )
    .with_resource_pack(true);
    let player = Arc::new(Player::new(
        profile,
        Arc::new(PlayerConnection::Java(connection)),
        Arc::clone(&session),
        running.player.get_world(),
        Arc::downgrade(&running.server),
        Arc::clone(&running.server.config),
        id,
        ClientInformation::default(),
    ));
    if !session.bind_initial_player(&player) {
        return Err("session refused the packed peer".into());
    }
    running.server.queue_player_join(Arc::clone(&player));
    common::until(&player, "packed peer admission", || {
        player.has_joined_world() && !player.is_domain_switching()
    })
    .await?;
    let mut received = Vec::new();
    receive_displays(
        &mut packets,
        &mut received,
        "assembled chest displays",
        |received| {
            displayed_chests(received, smaller).len() == 1 && active_displays(received).len() == 2
        },
    )
    .await?;
    assert_two_displays(&received, larger, smaller);

    let wall_in_neighbor = SMALL_CORE.offset(1, 0, 0);
    let parts = [
        (wall_in_neighbor, block("bulk-chest:wall")?, 6, 1),
        (SMALL_CORE, block("bulk-chest:core")?, 7, 0),
    ];
    for (part, state, walls, cores) in parts {
        let world = player.get_world();
        common::on_tick(&running.server, move || {
            world.set_block(
                part,
                vanilla_blocks::AIR.default_state(),
                UpdateFlags::UPDATE_ALL,
            );
            Ok(())
        })
        .await?;
        receive_displays(
            &mut packets,
            &mut received,
            "restored construction textures",
            |received| {
                displayed_chests(received, smaller).is_empty()
                    && displayed_construction(received, "bulk-chest:wall").len() == walls
                    && displayed_construction(received, "bulk-chest:core").len() == cores
            },
        )
        .await?;
        assert_eq!(active_displays(&received).len(), walls + cores + 1);
        common::until(
            &running.player,
            "one display on the client without the pack",
            || active_displays(&running.sent_packets.lock()).len() == 1,
        )
        .await?;

        let world = player.get_world();
        common::on_tick(&running.server, move || {
            world.set_block(part, state, UpdateFlags::UPDATE_ALL);
            Ok(())
        })
        .await?;
        receive_displays(
            &mut packets,
            &mut received,
            "reassembled chest displays",
            |received| {
                displayed_chests(received, smaller).len() == 1
                    && active_displays(received).len() == 2
            },
        )
        .await?;
        assert_two_displays(&received, larger, smaller);
    }
    Ok(())
}

async fn receive_displays(
    packets: &mut mpsc::UnboundedReceiver<OutboundPacket>,
    received: &mut Vec<EncodedPacket>,
    label: &str,
    ready: impl Fn(&[EncodedPacket]) -> bool,
) -> Result<(), String> {
    timeout(Duration::from_secs(5), async {
        while let Some(OutboundPacket::Packet(packet)) = packets.recv().await {
            received.push(packet);
            if ready(received) {
                while let Ok(OutboundPacket::Packet(packet)) = packets.try_recv() {
                    received.push(packet);
                }
                return Ok(());
            }
        }
        Err(format!(
            "packed peer disconnected while waiting for {label}"
        ))
    })
    .await
    .map_err(|_| {
        format!(
            "timed out waiting for {label}. Active displays: {}, walls: {}, cores: {}",
            active_displays(received).len(),
            displayed_construction(received, "bulk-chest:wall").len(),
            displayed_construction(received, "bulk-chest:core").len(),
        )
    })?
}

fn assert_two_displays(
    packets: &[EncodedPacket],
    larger: &ChestDisplayFields,
    smaller: &ChestDisplayFields,
) {
    let mut expected = displayed_chests(packets, larger);
    expected.extend(displayed_chests(packets, smaller));
    assert_eq!(expected.len(), 2);
    let actual = active_displays(packets);
    assert_eq!(
        actual.len(),
        expected.len(),
        "two assembled chests should send exactly two active item displays",
    );
    assert_eq!(actual, expected);
}

fn go_to_target_page(player: &Player, mut id: u8) -> Result<u8, String> {
    for _ in 1..TARGET_PAGE {
        click(player, id, NEXT_PAGE_BUTTON, ClickType::Pickup);
        let next = player
            .open_container_id()
            .ok_or("page switch closed the chest")?;
        assert_ne!(next, id);
        id = next;
    }
    Ok(id)
}

fn block(key: &str) -> Result<BlockStateId, String> {
    let key = key.parse().map_err(str::to_owned)?;
    REGISTRY
        .blocks
        .by_key(&key)
        .map(|block| block.default_state())
        .ok_or_else(|| format!("unregistered block {key}"))
}

fn item(key: &str) -> Result<ItemRef, String> {
    let key = key.parse().map_err(str::to_owned)?;
    REGISTRY
        .items
        .by_key(&key)
        .ok_or_else(|| format!("unregistered item {key}"))
}

fn place_part(player: &Arc<Player>, pos: BlockPos, against_face: Direction, item: ItemRef) {
    player
        .inventory
        .lock()
        .set_selected_item(ItemStack::new(item));
    click_block(player, against_face.opposite().relative(pos), against_face);
    player
        .inventory
        .lock()
        .set_selected_item(ItemStack::empty());
}

fn use_part(player: &Arc<Player>, pos: BlockPos) {
    click_block(player, pos, Direction::North);
}

fn click_block(player: &Arc<Player>, pos: BlockPos, direction: Direction) {
    player.handle_use_item_on(SUseItemOn {
        hand: InteractionHand::MainHand,
        block_hit: BlockHitResult {
            location: pos.get_center(),
            direction,
            block_pos: pos,
            miss: false,
            inside: false,
            world_border_hit: false,
        },
        sequence: 1,
    });
}

struct ChestDisplayFields {
    chest: Vec<u8>,
    scale: Vec<u8>,
}

fn chest_display_fields(edge: i32) -> Result<ChestDisplayFields, String> {
    let mut data = ItemDisplayEntityData::new();
    data.item_stack.set(ItemStack::new(&vanilla_items::CHEST));
    const BODY_EXTENT: f32 = 14.0 / 16.0;
    const EXTRA_WIDTH: f32 = 0.02;
    data.display
        .scale
        .set(Vec3::splat((edge as f32 + EXTRA_WIDTH) / BODY_EXTENT));
    let fields = data.pack_all();
    let index = fields.iter()
        .position(|field| matches!(&field.value, EntityData::ItemStack(stack) if stack.is(&vanilla_items::CHEST)))
        .ok_or("missing chest item display metadata")?;
    let scale_field = fields
        .iter()
        .find(|field| matches!(field.value, EntityData::Vector3(_)))
        .ok_or("missing scale metadata for the expected chest")?;
    let mut chest = Vec::new();
    let mut scale = Vec::new();
    write_data_values(&fields[index..], &mut chest).map_err(|error| error.to_string())?;
    scale_field
        .write_to(&mut scale)
        .map_err(|error| error.to_string())?;
    Ok(ChestDisplayFields { chest, scale })
}

fn chest_displays(running: &common::Running, fields: &ChestDisplayFields) -> BTreeSet<i32> {
    displayed_chests(&running.sent_packets.lock(), fields)
}

fn displayed_chests(packets: &[EncodedPacket], fields: &ChestDisplayFields) -> BTreeSet<i32> {
    tracked_displays(packets, |packet_id, payload| {
        if packet_id != C_SET_ENTITY_DATA || !payload.get_ref().ends_with(&fields.chest) {
            return None;
        }
        let matches_scale = payload
            .get_ref()
            .windows(fields.scale.len())
            .any(|bytes| bytes == fields.scale);
        if !matches_scale {
            return None;
        }
        VarInt::read(payload).ok().map(|id| id.0)
    })
}

fn displayed_construction(packets: &[EncodedPacket], model: &str) -> BTreeSet<i32> {
    tracked_displays(packets, |packet_id, payload| {
        if packet_id != C_SET_ENTITY_DATA
            || !payload
                .get_ref()
                .windows(model.len())
                .any(|bytes| bytes == model.as_bytes())
        {
            return None;
        }
        VarInt::read(payload).ok().map(|id| id.0)
    })
}

fn active_displays(packets: &[EncodedPacket]) -> BTreeSet<i32> {
    tracked_displays(packets, |packet_id, payload| {
        if packet_id != C_ADD_ENTITY {
            return None;
        }
        let id = VarInt::read(payload).ok()?.0;
        let _ = uuid::Uuid::read(payload).ok()?;
        let kind = VarInt::read(payload).ok()?.0;
        (kind == vanilla_entities::ITEM_DISPLAY.id() as i32).then_some(id)
    })
}

fn tracked_displays(
    packets: &[EncodedPacket],
    mut add: impl FnMut(i32, &mut Cursor<&[u8]>) -> Option<i32>,
) -> BTreeSet<i32> {
    let mut displays = BTreeSet::new();
    for packet in packets {
        let mut payload = Cursor::new(packet.encoded_data.as_slice());
        if VarInt::read(&mut payload).is_err() {
            continue;
        }
        let Ok(VarInt(packet_id)) = VarInt::read(&mut payload) else {
            continue;
        };
        if packet_id == C_REMOVE_ENTITIES {
            let Ok(VarInt(count)) = VarInt::read(&mut payload) else {
                continue;
            };
            for _ in 0..count {
                if let Ok(VarInt(id)) = VarInt::read(&mut payload) {
                    displays.remove(&id);
                }
            }
        } else if let Some(id) = add(packet_id, &mut payload) {
            displays.insert(id);
        }
    }
    displays
}

fn click(player: &Player, id: u8, slot: i16, click_type: ClickType) {
    player.handle_container_click(SContainerClick {
        container_id: i32::from(id),
        state_id: 0,
        slot_num: slot,
        button_num: 0,
        click_type,
        changed_slots: FxHashMap::default(),
        carried_item: HashedStack::Empty,
    });
}

fn current_menu(running: &common::Running, expected: &str) -> Result<u8, String> {
    let id = running.player.open_container_id().ok_or_else(|| {
        let messages: Vec<_> = running
            .sent_packets
            .lock()
            .iter()
            .filter_map(|packet| {
                let mut payload = Cursor::new(packet.encoded_data.as_slice());
                VarInt::read(&mut payload).ok()?;
                if VarInt::read(&mut payload).ok()?.0 != C_SYSTEM_CHAT {
                    return None;
                }
                TextComponent::read(&mut payload).ok()
            })
            .collect();
        format!("storage menu {expected} did not open, messages {messages:?}")
    })?;
    let sent = running.sent_packets.lock();
    let title = sent
        .iter()
        .rev()
        .find_map(|packet| {
            let mut payload = Cursor::new(packet.encoded_data.as_slice());
            VarInt::read(&mut payload).ok()?;
            if VarInt::read(&mut payload).ok()?.0 != C_OPEN_SCREEN
                || VarInt::read(&mut payload).ok()?.0 != i32::from(id)
            {
                return None;
            }
            VarInt::read(&mut payload).ok()?;
            TextComponent::read(&mut payload).ok()
        })
        .ok_or("missing storage screen packet")?;
    assert_eq!(title, TextComponent::plain(expected.to_owned()));
    Ok(id)
}
