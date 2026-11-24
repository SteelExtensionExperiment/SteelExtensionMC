//! Generated native UI, pack-aware menu packets, and vanilla inventory clicks.
#![cfg(feature = "host")]
#![recursion_limit = "256"]

mod common;

use std::{io::Cursor, sync::Arc, time::Duration};

use rustc_hash::FxHashMap;
use steel_core::{
    entity::next_entity_id,
    player::{
        ClientInformation, Player, PlayerConnection, PlayerSession,
        connection::{JavaConnection, OutboundPacket},
    },
    server::Server,
};
use steel_protocol::{
    packet_traits::EncodedPacket,
    packets::game::{ClickType, HashedStack, SContainerClick},
};
use steel_registry::{
    REGISTRY, RegistryExt,
    data_components::{
        DataComponentPatch,
        vanilla_components::{CUSTOM_NAME, ITEM_MODEL, LORE},
    },
    game_profile::GameProfile,
    item_stack::ItemStack,
    packets::play::{C_CONTAINER_SET_CONTENT, C_CONTAINER_SET_SLOT, C_OPEN_SCREEN},
    vanilla_items,
};
use steel_utils::{codec::VarInt, locks::AsyncMutex, serial::ReadFrom};
use text_components::{Modifier, TextComponent, format::Color};
use tokio::{sync::mpsc, time::timeout};
use tokio_util::sync::CancellationToken;

const START_ICON_SLOT: i16 = 30;
const SECOND_START_ICON_SLOT: usize = 31;
const START_EDGE_SLOT: i16 = 41;
const LABEL_SLOT: i16 = 2;
const SCROLL_DOWN_SLOT: i16 = 44;
const FIRST_RECIPE_SLOT: i16 = 16;
const INPUT_SLOT: i16 = 11;
const ENERGY_BOTTOM_SLOT: i16 = 36;
const LAST_PROGRESS_SLOT: i16 = 23;
const STATUS_SLOT: i16 = 47;
const CLOSE_SLOT: i16 = 53;
const START_MODEL: &str = "ui-demo:ui/furnace/start-flame";
const REACTOR_FONT: &str = "ui-demo:ui/reactor";
const REACTOR_RUN_EDGE: i16 = 32;
const REACTOR_CLOSE: i16 = 7;
const REACTOR_OUTPUT: i16 = 15;
const REACTOR_HEAT_TOP: i16 = 17;
const REACTOR_HEAT_MIDDLE: i16 = 26;
const REACTOR_ENERGY_MIDDLE: i16 = 18;
const GAUGE_LEVELS: usize = 15;

#[test]
#[ignore = "requires cargo build --manifest-path extension-example/ui/Cargo.toml --workspace"]
fn ui_demos_animate_and_route_buttons_with_both_pack_modes() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let loaded = common::load_extensions(
        &[("steel_ui_demo", ""), ("steel_ui_extension", "")],
        dir.path(),
    )?;
    assert_eq!(loaded.order, ["steel-ui", "steel-ui-demo"]);
    let assets = REGISTRY.client_content.resource_pack();
    assert!(
        assets
            .files()
            .any(|(path, _)| { path == "assets/ui-demo/textures/item/ui/furnace/start-flame.png" })
    );
    assert!(
        assets
            .files()
            .any(|(path, _)| { path == "assets/ui-demo/items/ui/furnace/energy-pulse.json" })
    );
    let font = assets
        .files()
        .find(|(path, _)| *path == "assets/ui-demo/font/ui/reactor.json")
        .ok_or("reactor font was not registered")?;
    let font: serde_json::Value =
        serde_json::from_slice(font.1).map_err(|error| error.to_string())?;
    assert_eq!(font["providers"][0]["advances"]["\u{e000}"], -8);
    assert_eq!(font["providers"][1]["file"], "ui-demo:gui/ui/reactor.png");
    assert_eq!(font["providers"][1]["chars"][0], "\u{e001}");
    assert!(
        assets
            .files()
            .any(|(path, _)| path == "assets/ui-demo/textures/gui/ui/reactor.png")
    );
    for gauge in ["energy-fill", "heat-fill"] {
        for level in [1, GAUGE_LEVELS] {
            let path = format!("assets/ui-demo/items/ui/reactor/{gauge}-{level}.json");
            let (_, definition) = assets
                .files()
                .find(|(candidate, _)| *candidate == path)
                .ok_or_else(|| format!("missing gauge definition {path}"))?;
            let definition: serde_json::Value =
                serde_json::from_slice(definition).map_err(|error| error.to_string())?;
            assert_eq!(definition["oversized_in_gui"], true);
            let model_path = format!("assets/ui-demo/models/item/ui/reactor/{gauge}-{level}.json");
            let (_, model) = assets
                .files()
                .find(|(candidate, _)| *candidate == model_path)
                .ok_or_else(|| format!("missing gauge model {model_path}"))?;
            let model: serde_json::Value =
                serde_json::from_slice(model).map_err(|error| error.to_string())?;
            assert_eq!(
                model["textures"]["fill"],
                format!("ui-demo:item/ui/reactor/{gauge}")
            );
            assert_eq!(
                model["elements"][0]["faces"]["north"]["uv"],
                serde_json::json!([16, level, 0, level + 1])
            );
        }
    }
    let fixture = common::Fixture::new()?;
    fixture.runtime.block_on(async {
        let uuid = uuid::Uuid::new_v4();
        let running = common::start(
            &fixture.world,
            (loaded.extensions, loaded.commands),
            common::grants(uuid, &[])?,
            uuid,
            "UiTester",
        )
        .await?;
        let result = async {
            check_screen(&running).await?;
            check_reactor_fallback(&running).await?;
            check_packed_reactor(&running).await
        }
        .await;
        common::stop(running, &loaded.initialized, &fixture.world).await?;
        result
    })
}

async fn check_reactor_fallback(running: &common::Running) -> Result<(), String> {
    let player = &running.player;
    common::command(&running.server, player, "ui-demo reactor").await?;
    common::until(player, "fallback reactor", || {
        player.open_container_id().is_some()
    })
    .await?;
    let container = i32::from(player.open_container_id().ok_or("reactor is closed")?);
    let title = menu_title(running, container).ok_or("reactor title was not sent")?;
    assert_eq!(
        title,
        TextComponent::plain("Pulse Reactor")
            .color(Color::Rgb(0x22, 0x37, 0x40))
            .italic(false),
    );
    send_click(
        &running.server,
        player,
        container,
        REACTOR_RUN_EDGE,
        ClickType::Pickup,
    )
    .await?;
    common::until(player, "reactor copper output", || {
        slot_update(running, container, REACTOR_OUTPUT, false, |item| {
            item.is(&vanilla_items::COPPER_INGOT)
        })
    })
    .await?;
    assert!(slot_update(
        running,
        container,
        REACTOR_HEAT_TOP,
        false,
        |item| item.is(&vanilla_items::ORANGE_STAINED_GLASS_PANE)
    ));
    send_click(
        &running.server,
        player,
        container,
        REACTOR_CLOSE,
        ClickType::Pickup,
    )
    .await?;
    common::until(player, "reactor close", || {
        player.open_container_id().is_none()
    })
    .await
}

async fn check_packed_reactor(running: &common::Running) -> Result<(), String> {
    let server = &running.server;
    let profile = GameProfile::new(uuid::Uuid::new_v4(), "PackedUi".into(), Vec::new())
        .map_err(|error| error.to_string())?;
    let session = Arc::new(PlayerSession::new(10, 10));
    let connection_id = next_entity_id();
    let (outgoing, mut packets) = mpsc::unbounded_channel();
    let connection = JavaConnection::new(
        outgoing,
        CancellationToken::new(),
        None,
        Arc::new(AsyncMutex::new(None)),
        u64::try_from(connection_id).map_err(|error| error.to_string())?,
        Arc::clone(&session),
    )
    .with_resource_pack(true);
    let player = Arc::new(Player::new(
        profile,
        Arc::new(PlayerConnection::Java(connection)),
        Arc::clone(&session),
        running.player.get_world(),
        Arc::downgrade(server),
        Arc::clone(&server.config),
        connection_id,
        ClientInformation::default(),
    ));
    if !session.bind_initial_player(&player) {
        return Err("session refused the packed peer".into());
    }
    server.queue_player_join(Arc::clone(&player));
    common::until(&player, "packed peer admission", || {
        player.has_joined_world() && !player.is_domain_switching()
    })
    .await?;
    common::command(server, &player, "ui-demo reactor").await?;
    let (container, title) =
        receive_packet(&mut packets, "packed reactor title", read_menu_title).await?;
    assert_eq!(
        title,
        TextComponent::plain("\u{e000}\u{e001}")
            .font(REACTOR_FONT)
            .color(Color::Rgb(0xff, 0xff, 0xff))
    );
    check_gauge_frames(
        &mut packets,
        container,
        REACTOR_ENERGY_MIDDLE,
        "energy-fill",
        false,
    )
    .await?;
    receive_packet(&mut packets, "energy reset", |packet| {
        read_slot_update(packet, container, REACTOR_ENERGY_MIDDLE).filter(ItemStack::is_empty)
    })
    .await?;
    send_click(
        server,
        &player,
        container,
        REACTOR_RUN_EDGE,
        ClickType::Pickup,
    )
    .await?;
    check_gauge_frames(
        &mut packets,
        container,
        REACTOR_HEAT_MIDDLE,
        "heat-fill",
        true,
    )
    .await?;
    Ok(())
}

async fn check_gauge_frames(
    packets: &mut mpsc::UnboundedReceiver<OutboundPacket>,
    container: i32,
    slot: i16,
    name: &str,
    expect_output: bool,
) -> Result<(), String> {
    let mut next_level = 1;
    let mut saw_output = !expect_output;
    receive_packet(packets, name, |packet| {
        if next_level <= GAUGE_LEVELS
            && let Some(item) = read_slot_update(packet, container, slot)
        {
            let model = format!("ui-demo:ui/reactor/{name}-{next_level}");
            if item
                .get(ITEM_MODEL)
                .is_some_and(|key| key.to_string() == model)
            {
                next_level += 1;
            }
        }
        if !saw_output {
            saw_output = read_slot_update(packet, container, REACTOR_OUTPUT)
                .is_some_and(|item| item.is(&vanilla_items::COPPER_INGOT));
        }
        (saw_output && next_level > GAUGE_LEVELS).then_some(())
    })
    .await
}

async fn receive_packet<T>(
    packets: &mut mpsc::UnboundedReceiver<OutboundPacket>,
    label: &str,
    mut read: impl FnMut(&EncodedPacket) -> Option<T>,
) -> Result<T, String> {
    timeout(Duration::from_secs(2), async {
        while let Some(packet) = packets.recv().await {
            if let OutboundPacket::Packet(packet) = packet
                && let Some(value) = read(&packet)
            {
                return Some(value);
            }
        }
        None
    })
    .await
    .map_err(|error| format!("timed out waiting for {label}: {error}"))?
    .ok_or_else(|| format!("connection closed while waiting for {label}"))
}

async fn check_screen(running: &common::Running) -> Result<(), String> {
    let player = &running.player;
    common::command(&running.server, player, "ui-demo open").await?;
    common::until(player, "compiled UI menu", || {
        player.open_container_id().is_some()
    })
    .await?;
    let container = i32::from(player.open_container_id().ok_or("UI menu is closed")?);
    common::until(player, "compiled UI menu content", || {
        menu_packet(running, container).is_some()
    })
    .await?;
    let packet = menu_packet(running, container).ok_or("UI menu content was not sent")?;
    let title = menu_title(running, container).ok_or("UI menu title was not sent")?;
    assert_eq!(
        title,
        TextComponent::plain("Electric Furnace")
            .color(Color::Rgb(0x24, 0x3f, 0x45))
            .italic(false),
    );
    let slot = usize::try_from(START_ICON_SLOT).map_err(|error| error.to_string())?;
    let plain = read_slot(&packet.clone().for_resource_pack(false), slot)
        .map_err(|error| error.to_string())?;
    let packed = packet.for_resource_pack(true);
    let themed = read_slot(&packed, slot).map_err(|error| error.to_string())?;
    let second_icon =
        read_slot(&packed, SECOND_START_ICON_SLOT).map_err(|error| error.to_string())?;
    assert!(plain.is(&vanilla_items::BLAZE_POWDER));
    assert!(themed.is(&vanilla_items::BLAZE_POWDER));
    assert_eq!(
        plain.get(ITEM_MODEL),
        vanilla_items::BLAZE_POWDER
            .components
            .get(ITEM_MODEL)
            .as_ref(),
    );
    assert_eq!(
        themed.get(ITEM_MODEL).map(ToString::to_string).as_deref(),
        Some(START_MODEL)
    );
    assert_eq!(second_icon.get(ITEM_MODEL), themed.get(ITEM_MODEL));
    assert_eq!(plain.get(CUSTOM_NAME), themed.get(CUSTOM_NAME));
    assert_eq!(plain.get(LORE), themed.get(LORE));
    assert_eq!(
        plain.get(CUSTOM_NAME),
        Some(&TextComponent::plain("Start cycle").italic(false)),
    );
    assert_eq!(
        plain.get(LORE).map(|lore| lore.lines()),
        Some([TextComponent::plain("Run the selected recipe preview.").italic(false)].as_slice()),
    );

    for (slot, kind) in [
        (LABEL_SLOT, ClickType::Pickup),
        (START_ICON_SLOT, ClickType::QuickMove),
    ] {
        send_click(&running.server, player, container, slot, kind).await?;
        assert!(
            player.open_container_id().is_some(),
            "a non-button input closed the furnace"
        );
    }
    let running_status = TextComponent::plain("Smelting preview").content;
    assert!(!menu_sync(running, container, STATUS_SLOT, |item| {
        item.get(CUSTOM_NAME)
            .is_some_and(|name| name.content == running_status)
    }));
    send_click(
        &running.server,
        player,
        container,
        SCROLL_DOWN_SLOT,
        ClickType::Pickup,
    )
    .await?;
    common::until(player, "recipe list scroll", || {
        menu_sync(running, container, FIRST_RECIPE_SLOT, |item| {
            item.is(&vanilla_items::GOLD_INGOT)
        })
    })
    .await?;
    send_click(
        &running.server,
        player,
        container,
        FIRST_RECIPE_SLOT,
        ClickType::Pickup,
    )
    .await?;
    common::until(player, "scrolled recipe selection", || {
        menu_sync(running, container, INPUT_SLOT, |item| {
            item.is(&vanilla_items::RAW_GOLD)
        })
    })
    .await?;
    send_click(
        &running.server,
        player,
        container,
        START_EDGE_SLOT,
        ClickType::Pickup,
    )
    .await?;
    common::until(
        player,
        "completed cycle from the edge of the button",
        || {
            slot_update(running, container, LAST_PROGRESS_SLOT, false, |item| {
                item.is(&vanilla_items::YELLOW_STAINED_GLASS_PANE)
            })
        },
    )
    .await?;
    for style in ["energy-pulse", "energy-on", "energy-off"] {
        let model = format!("ui-demo:ui/furnace/{style}");
        common::until(player, style, || {
            slot_update(running, container, ENERGY_BOTTOM_SLOT, true, |item| {
                item.get(ITEM_MODEL)
                    .is_some_and(|key| key.to_string() == model)
            })
        })
        .await?;
    }
    assert!(slot_update(
        running,
        container,
        ENERGY_BOTTOM_SLOT,
        false,
        |item| { item.is(&vanilla_items::LIGHT_BLUE_STAINED_GLASS_PANE) }
    ));
    send_click(
        &running.server,
        player,
        container,
        CLOSE_SLOT,
        ClickType::Pickup,
    )
    .await?;
    common::until(player, "furnace menu close", || {
        player.open_container_id().is_none()
    })
    .await?;
    assert!(
        !player
            .inventory
            .lock()
            .get_items()
            .iter()
            .any(|item| item.is(&vanilla_items::GOLD_INGOT))
    );
    Ok(())
}

fn menu_sync(
    running: &common::Running,
    container: i32,
    slot: i16,
    matches: impl Fn(&ItemStack) -> bool,
) -> bool {
    let Ok(slot) = usize::try_from(slot) else {
        return false;
    };
    running.sent_packets.lock().iter().any(|packet| {
        let Some(mut payload) = packet_payload(packet, C_CONTAINER_SET_CONTENT) else {
            return false;
        };
        VarInt::read(&mut payload).ok().map(|value| value.0) == Some(container)
            && read_slot(packet, slot).is_ok_and(|item| matches(&item))
    })
}

fn slot_update(
    running: &common::Running,
    container: i32,
    slot: i16,
    packed: bool,
    matches: impl Fn(&ItemStack) -> bool,
) -> bool {
    running.sent_packets.lock().iter().any(|packet| {
        let packet = packet.clone().for_resource_pack(packed);
        read_slot_update(&packet, container, slot).is_some_and(|item| matches(&item))
    })
}

fn read_slot_update(packet: &EncodedPacket, container: i32, slot: i16) -> Option<ItemStack> {
    let mut payload = packet_payload(packet, C_CONTAINER_SET_SLOT)?;
    let packet_container = VarInt::read(&mut payload).ok()?.0;
    let _state = VarInt::read(&mut payload).ok()?;
    let packet_slot = i16::read(&mut payload).ok()?;
    if packet_container != container || packet_slot != slot {
        return None;
    }
    read_client_item(&mut payload).ok()
}

fn menu_packet(running: &common::Running, container: i32) -> Option<EncodedPacket> {
    running.sent_packets.lock().iter().find_map(|packet| {
        let mut payload = packet_payload(packet, C_CONTAINER_SET_CONTENT)?;
        (VarInt::read(&mut payload).ok()?.0 == container).then(|| packet.clone())
    })
}

fn menu_title(running: &common::Running, container: i32) -> Option<TextComponent> {
    running.sent_packets.lock().iter().find_map(|packet| {
        let (id, title) = read_menu_title(packet)?;
        (id == container).then_some(title)
    })
}

fn read_menu_title(packet: &EncodedPacket) -> Option<(i32, TextComponent)> {
    let mut payload = packet_payload(packet, C_OPEN_SCREEN)?;
    let container = VarInt::read(&mut payload).ok()?.0;
    let _menu_type = VarInt::read(&mut payload).ok()?;
    Some((container, TextComponent::read(&mut payload).ok()?))
}

fn packet_payload(packet: &EncodedPacket, id: i32) -> Option<Cursor<&[u8]>> {
    let mut payload = Cursor::new(packet.encoded_data.as_slice());
    VarInt::read(&mut payload).ok()?;
    (VarInt::read(&mut payload).ok()?.0 == id).then_some(payload)
}

fn read_slot(packet: &EncodedPacket, target: usize) -> std::io::Result<ItemStack> {
    let mut payload = packet_payload(packet, C_CONTAINER_SET_CONTENT)
        .ok_or_else(|| std::io::Error::other("missing menu packet"))?;
    let _container = VarInt::read(&mut payload)?;
    let _state = VarInt::read(&mut payload)?;
    let slots = usize::try_from(VarInt::read(&mut payload)?.0).map_err(std::io::Error::other)?;
    for slot in 0..slots {
        let item = read_client_item(&mut payload)?;
        if slot == target {
            return Ok(item);
        }
    }
    Err(std::io::Error::other("no icon in the target slot"))
}

fn read_client_item(payload: &mut Cursor<&[u8]>) -> std::io::Result<ItemStack> {
    let count = VarInt::read(payload)?.0;
    if count == 0 {
        return Ok(ItemStack::empty());
    }
    let id = usize::try_from(VarInt::read(payload)?.0).map_err(std::io::Error::other)?;
    let patch = DataComponentPatch::read(payload)?;
    let item = REGISTRY
        .items
        .by_id(id)
        .ok_or_else(|| std::io::Error::other("invalid item id"))?;
    Ok(ItemStack::with_count_and_patch(item, count, patch))
}

async fn send_click(
    server: &Arc<Server>,
    player: &Arc<Player>,
    container: i32,
    slot: i16,
    kind: ClickType,
) -> Result<(), String> {
    let player = Arc::clone(player);
    common::on_tick(server, move || {
        player.handle_container_click(SContainerClick {
            container_id: container,
            state_id: 0,
            slot_num: slot,
            button_num: 0,
            click_type: kind,
            changed_slots: FxHashMap::default(),
            carried_item: HashedStack::Empty,
        });
        Ok(())
    })
    .await
}
