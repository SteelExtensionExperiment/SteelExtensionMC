//! Observe the two HUD surfaces as a vanilla peer receives their updates.
use super::*;
use std::{collections::BTreeMap, io::Cursor, time::Duration};
use steel_core::{
    entity::{SharedEntity, next_entity_id},
    player::{
        ClientInformation, PlayerConnection, PlayerSession,
        connection::{JavaConnection, OutboundPacket},
        hud::HudPolicy,
    },
    world::World,
};
use steel_registry::{
    game_profile::GameProfile,
    packets::play::{C_BOSS_EVENT, C_RESET_SCORE, C_SET_ACTION_BAR_TEXT, C_SET_SCORE, C_TAB_LIST},
};
use steel_utils::{
    codec::VarInt,
    locks::AsyncMutex,
    serial::{PrefixedRead, ReadFrom},
    text::DisplayResolutor,
};
use text_components::{TextComponent, format::Color};
use tokio::{sync::mpsc, time::timeout};
use tokio_util::sync::CancellationToken;

const MANA_FONT: &str = "magic:hud/mana";
const PARTY_FONT: &str = "creatures:hud/party";

fn contains(bytes: &[u8], text: &str) -> bool {
    bytes
        .windows(text.len())
        .any(|part| part == text.as_bytes())
}

fn sidebar(running: &common::Running) -> Result<Vec<Vec<u8>>, String> {
    let mut lines = BTreeMap::new();
    for packet in running.sent_packets.lock().iter() {
        let mut cursor = Cursor::new(packet.encoded_data.as_slice());
        let length = VarInt::read(&mut cursor)
            .map_err(|error| error.to_string())?
            .0;
        assert!(length > 0);
        let id = VarInt::read(&mut cursor)
            .map_err(|error| error.to_string())?
            .0;
        if id == C_BOSS_EVENT {
            assert!(
                !contains(packet.encoded_data.as_slice(), "Mana"),
                "mana must not publish a boss bar"
            );
        }
        if id != C_SET_SCORE && id != C_RESET_SCORE {
            continue;
        }
        let owner =
            String::read_prefixed::<VarInt>(&mut cursor).map_err(|error| error.to_string())?;
        if id == C_RESET_SCORE {
            lines.remove(&owner);
            continue;
        }
        let objective =
            String::read_prefixed::<VarInt>(&mut cursor).map_err(|error| error.to_string())?;
        if objective != "steel_sidebar" {
            continue;
        }
        let score = VarInt::read(&mut cursor)
            .map_err(|error| error.to_string())?
            .0;
        let offset = usize::try_from(cursor.position()).map_err(|error| error.to_string())?;
        lines.insert(owner, (score, packet.encoded_data[offset..].to_vec()));
    }
    // Vanilla PlayerScoreEntry.isHidden excludes these holders before sorting.
    let mut lines: Vec<_> = lines
        .into_iter()
        .filter(|(owner, _)| !owner.starts_with('#'))
        .map(|(_, line)| line)
        .collect();
    lines.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    Ok(lines.into_iter().map(|(_, text)| text).collect())
}

fn displayed_text(bytes: &[u8]) -> Result<TextComponent, String> {
    let mut cursor = Cursor::new(bytes);
    if !bool::read(&mut cursor).map_err(|error| error.to_string())? {
        return Err("sidebar score has no display text".into());
    }
    let tag = simdnbt::owned::read_tag(&mut cursor).map_err(|error| format!("{error:?}"))?;
    TextComponent::try_from_nbt(&tag).map_err(|error| error.to_string())
}

fn action_bar(running: &common::Running) -> Result<TextComponent, String> {
    for packet in running.sent_packets.lock().iter().rev() {
        let mut cursor = Cursor::new(packet.encoded_data.as_slice());
        VarInt::read(&mut cursor).map_err(|error| error.to_string())?;
        let VarInt(id) = VarInt::read(&mut cursor).map_err(|error| error.to_string())?;
        if id == C_SET_ACTION_BAR_TEXT {
            let tag =
                simdnbt::owned::read_tag(&mut cursor).map_err(|error| format!("{error:?}"))?;
            return TextComponent::try_from_nbt(&tag).map_err(|error| error.to_string());
        }
    }
    Err("action-bar packet missing".into())
}

async fn next_roster_update(running: &common::Running, world: &World) -> Result<(), String> {
    let started = world.game_time();
    common::until(&running.player, "roster refresh", || {
        world.game_time() - started > 20
    })
    .await
}

pub(super) async fn shared_scoreboard(
    running: &common::Running,
    world: &Arc<World>,
) -> Result<(), String> {
    let actor = Arc::clone(&running.player);
    let level = Arc::clone(world);
    let health = common::on_tick(&running.server, move || {
        let mut health = Vec::new();
        for (slot, name) in ["Moss", "Cloud", "Clover", "Hidden"]
            .into_iter()
            .enumerate()
        {
            let path = format!("creatures:state/party/{slot}");
            actor.data().remove(&DataPath::parse(&path)?);
            let entity = ENTITIES
                .create(
                    &vanilla_entities::SHEEP,
                    DVec3::new(10.5, 65.0, 8.5),
                    Arc::downgrade(&level),
                )
                .ok_or("sheep factory missing")?;
            let living = entity.as_living_entity().ok_or("sheep is not living")?;
            let maximum = living.get_max_health();
            living.set_health(maximum * (slot + 1) as f32 / 4.0);
            health.push(format!("HP: {}/{maximum}", living.get_health()));
            level
                .try_add_entity(Arc::clone(&entity))
                .map_err(|error| error.to_string())?;
            store(&actor, &entity, &path)?;
            let mut data = actor.data();
            data.set(
                &DataPath::parse(&format!("{path}/label"))?,
                NbtTag::String(name.into()),
            )?;
            data.set(
                &DataPath::parse(&format!("{path}/grade"))?,
                NbtTag::Long((slot % 3) as i64),
            )?;
        }
        Ok(health)
    })
    .await?;
    next_roster_update(running, world).await?;
    let lines = sidebar(running)?;
    assert_eq!(lines.len(), 10, "sidebar must display three creatures");
    for (index, name) in ["Moss", "Cloud", "Clover"].into_iter().enumerate() {
        assert!(
            lines
                .iter()
                .any(|line| contains(line, &format!("{}. {name}", index + 1)))
        );
        assert!(lines.iter().any(|line| contains(line, &health[index])));
    }
    for ball in ["Poké Ball", "Super Ball", "Hyper Ball"] {
        assert!(
            lines
                .iter()
                .any(|line| contains(line, &format!("{ball} · In ball")))
        );
    }
    assert!(!lines.iter().any(|line| contains(line, "Hidden")));
    assert!(!lines.iter().any(|line| contains(line, "Mana:")));
    assert!(
        lines
            .iter()
            .all(|line| !contains(line, MANA_FONT) && !contains(line, PARTY_FONT))
    );
    for line in &lines {
        let text = displayed_text(line)?;
        let plain = text.to_plain(&DisplayResolutor);
        let expected = if plain.starts_with("HP:") {
            Some(Color::Green)
        } else if plain.starts_with("1. ") {
            Some(Color::Gold)
        } else {
            continue;
        };
        assert!(
            text.format.color == expected,
            "missing HUD color for {plain}"
        );
    }
    assert!(running.player.hud_conflicts().is_empty());

    let actor = Arc::clone(&running.player);
    let level = Arc::clone(world);
    let (deployed, health): (SharedEntity, String) = common::on_tick(&running.server, move || {
        let deployed = restore(
            &actor,
            &level,
            "creatures:state/party/1",
            DVec3::new(8.5, 65.0, 8.5),
        )?;
        let living = deployed.as_living_entity().ok_or("sheep is not living")?;
        living.heal(1.0);
        let health = format!("HP: {}/{}", living.get_health(), living.get_max_health());
        let mut data = actor.data();
        data.set(
            &DataPath::parse("magic:state/mana/maximum")?,
            NbtTag::Double(200.0),
        )?;
        data.set(
            &DataPath::parse("magic:state/mana/current")?,
            NbtTag::Double(50.0),
        )?;
        data.set(
            &DataPath::parse("creatures:state/party/0/label")?,
            NbtTag::String("Oak".into()),
        )?;
        Ok((deployed, health))
    })
    .await?;
    next_roster_update(running, world).await?;
    let lines = sidebar(running)?;
    for expected in ["1. Oak", health.as_str(), "Super Ball · Deployed"] {
        assert!(
            lines.iter().any(|line| contains(line, expected)),
            "missing updated sidebar text: {expected}"
        );
    }
    const EXPECTED_MANA: &str = "Mana: 50/200 █████░░░░░░░░░░░░░░░";
    common::until(&running.player, "updated mana action bar", || {
        action_bar(running)
            .is_ok_and(|text| text.to_plain(&DisplayResolutor).trim_end() == EXPECTED_MANA)
    })
    .await?;
    let mana = action_bar(running)?;
    let line = mana
        .children
        .first()
        .ok_or("missing mana action-bar line")?;
    let meter = line.children.first().ok_or("missing mana meter")?;
    assert_eq!(line.format.color, Some(Color::Aqua));
    assert_eq!(meter.format.color, Some(Color::Blue));
    assert!(meter.format.font.is_none());
    assert!(
        mana.to_plain(&DisplayResolutor).ends_with(' '),
        "the visible text must be centered toward the armor bar"
    );
    assert!(!lines.iter().any(|line| contains(line, "1. Moss")));
    assert!(running.player.hud_conflicts().is_empty());

    const NOTICE: &str = "Temporary notification";
    let actor = Arc::clone(&running.player);
    common::on_tick(&running.server, move || {
        actor.send_action_bar(TextComponent::plain(NOTICE));
        actor.data().set(
            &DataPath::parse("magic:state/mana/current")?,
            NbtTag::Double(100.0),
        )?;
        Ok(())
    })
    .await?;
    next_roster_update(running, world).await?;
    assert_eq!(action_bar(running)?.to_plain(&DisplayResolutor), NOTICE);
    common::until(&running.player, "mana after temporary notification", || {
        action_bar(running).is_ok_and(|text| {
            text.to_plain(&DisplayResolutor).trim_end() == "Mana: 100/200 ██████████░░░░░░░░░░"
        })
    })
    .await?;

    let actor = Arc::clone(&running.player);
    common::on_tick(&running.server, move || {
        return_all(&actor)?;
        assert!(deployed.is_removed());
        Ok(())
    })
    .await
}

pub(super) async fn packed_scoreboard(running: &common::Running) -> Result<(), String> {
    let pack = REGISTRY.client_content.resource_pack();
    let mana = [("magic:hud/mana.png", "█░")];
    let party = [
        ("minecraft:gui/sprites/hud/heart/full.png", "█"),
        ("minecraft:gui/sprites/hud/heart/container.png", "░"),
        ("creatures:item/poke_ball.png", "\u{e000}"),
        ("creatures:item/super_ball.png", "\u{e001}"),
        ("creatures:item/hyper_ball.png", "\u{e002}"),
    ];
    for (asset, artwork) in [
        ("assets/magic/font/hud/mana.json", mana.as_slice()),
        ("assets/creatures/font/hud/party.json", party.as_slice()),
    ] {
        let bytes = pack
            .files()
            .find_map(|(path, bytes)| (path == asset).then_some(bytes))
            .ok_or_else(|| format!("missing generated font {asset}"))?;
        let font: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let providers = font["providers"]
            .as_array()
            .ok_or("missing bitmap providers")?;
        assert_eq!(providers.len(), artwork.len());
        for (provider, &(texture, glyphs)) in providers.iter().zip(artwork) {
            assert_eq!(provider["type"], "bitmap");
            assert_eq!(provider["file"], texture);
            assert_eq!(provider["chars"], serde_json::json!([glyphs]));
            let (namespace, path) = texture.split_once(':').ok_or("texture needs a namespace")?;
            if namespace != "minecraft" {
                let png = format!("assets/{namespace}/textures/{path}");
                assert!(pack.files().any(|(path, _)| path == png), "missing {png}");
            }
        }
    }

    let (outgoing, mut incoming) = mpsc::unbounded_channel();
    let id = next_entity_id();
    let session = Arc::new(PlayerSession::new(10, 10));
    let connection = JavaConnection::new(
        outgoing,
        CancellationToken::new(),
        None,
        Arc::new(AsyncMutex::new(None)),
        u64::try_from(id).map_err(|error| error.to_string())?,
        Arc::clone(&session),
    )
    .with_resource_pack(true);
    let profile = GameProfile::new(uuid::Uuid::new_v4(), "PackedMage".into(), Vec::new())
        .map_err(|error| error.to_string())?;
    let mut config: toml::Table = toml::from_str(include_str!(
        "../../../doc/extensions/examples/combined-hud.toml"
    ))
    .map_err(|error| error.to_string())?;
    let policy: HudPolicy = config
        .remove("server")
        .and_then(|mut server| server.as_table_mut()?.remove("hud"))
        .ok_or("missing combined HUD policy")?
        .try_into()
        .map_err(|error: toml::de::Error| error.to_string())?;
    let mut runtime_config = running.server.config.as_ref().clone();
    runtime_config.hud.tab_footer = policy.tab_footer;
    runtime_config.hud.tab_header = Some(
        policy
            .sidebar_lines
            .into_iter()
            .flatten()
            .next()
            .ok_or("missing first creature row in the combined HUD")?,
    );
    let player = Arc::new(Player::new(
        profile,
        Arc::new(PlayerConnection::Java(connection)),
        Arc::clone(&session),
        running.player.get_world(),
        Arc::downgrade(&running.server),
        Arc::new(runtime_config),
        id,
        ClientInformation::default(),
    ));
    if !session.bind_initial_player(&player) {
        return Err("session refused the packed peer".into());
    }
    running.server.queue_player_join(Arc::clone(&player));
    common::until(&player, "packed mage admission", || {
        player.has_joined_world() && !player.is_domain_switching()
    })
    .await?;
    wait_for_meter(&mut incoming, "████████████████████").await?;

    let mage = Arc::clone(&player);
    common::on_tick(&running.server, move || {
        let mut data = mage.data();
        data.set(
            &DataPath::parse("magic:state/mana/maximum")?,
            NbtTag::Double(200.0),
        )?;
        data.set(
            &DataPath::parse("magic:state/mana/current")?,
            NbtTag::Double(50.0),
        )?;
        Ok(())
    })
    .await?;
    wait_for_meter(&mut incoming, "█████░░░░░░░░░░░░░░░").await?;

    let mage = Arc::clone(&player);
    common::on_tick(&running.server, move || {
        const SUPER_BALL_GRADE: i64 = 1;
        let world = mage.get_world();
        let creature = ENTITIES
            .create(
                &vanilla_entities::SHEEP,
                mage.position(),
                Arc::downgrade(&world),
            )
            .ok_or("sheep factory missing")?;
        let living = creature.as_living_entity().ok_or("sheep is not living")?;
        living.set_health(living.get_max_health() / 2.0);
        world
            .try_add_entity(Arc::clone(&creature))
            .map_err(|error| error.to_string())?;
        store(&mage, &creature, "creatures:state/party/0")?;
        let mut data = mage.data();
        data.set(
            &DataPath::parse("creatures:state/party/0/label")?,
            NbtTag::String("Rune".into()),
        )?;
        data.set(
            &DataPath::parse("creatures:state/party/0/grade")?,
            NbtTag::Long(SUPER_BALL_GRADE),
        )?;
        Ok(())
    })
    .await?;
    wait_for_hud(
        &mut incoming,
        &[
            (C_SET_SCORE, &[PARTY_FONT, "HP:", "██░░░"]),
            (
                C_SET_SCORE,
                &[PARTY_FONT, "\u{e001}", "Super Ball · In ball"],
            ),
            (C_TAB_LIST, &[PARTY_FONT, "Rune:", "██░░░"]),
        ],
    )
    .await?;
    let mage = Arc::clone(&player);
    common::on_tick(&running.server, move || {
        let maximum = number(&mage, "creatures:state/party/0/health/maximum")?;
        mage.data().set(
            &DataPath::parse("creatures:state/party/0/health/current")?,
            NbtTag::Double(maximum),
        )?;
        Ok(())
    })
    .await?;
    wait_for_hud(
        &mut incoming,
        &[(C_SET_SCORE, &[PARTY_FONT, "HP:", "█████"])],
    )
    .await?;
    assert!(player.hud_conflicts().is_empty());
    Ok(())
}

async fn wait_for_meter(
    packets: &mut mpsc::UnboundedReceiver<OutboundPacket>,
    meter: &str,
) -> Result<(), String> {
    wait_for_hud(
        packets,
        &[
            (C_SET_ACTION_BAR_TEXT, &["Mana:", MANA_FONT, meter]),
            (C_TAB_LIST, &[MANA_FONT, meter]),
        ],
    )
    .await
}

async fn wait_for_hud(
    packets: &mut mpsc::UnboundedReceiver<OutboundPacket>,
    expected: &[(i32, &[&str])],
) -> Result<(), String> {
    let mut pending = expected.to_vec();
    timeout(Duration::from_secs(5), async {
        while let Some(OutboundPacket::Packet(packet)) = packets.recv().await {
            let payload = packet.encoded_data.as_slice();
            let mut reader = Cursor::new(payload);
            if VarInt::read(&mut reader).is_err() {
                continue;
            }
            let Ok(VarInt(id)) = VarInt::read(&mut reader) else {
                continue;
            };
            pending.retain(|(kind, text)| {
                *kind != id || !text.iter().all(|text| contains(payload, text))
            });
            if pending.is_empty() {
                return Ok(());
            }
        }
        Err("packed peer disconnected before the HUD arrived".into())
    })
    .await
    .map_err(|_| format!("timed out waiting for packed HUD {pending:?}"))?
}
