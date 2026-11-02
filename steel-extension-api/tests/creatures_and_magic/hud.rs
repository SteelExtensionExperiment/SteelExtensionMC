//! Observe the shared sidebar as a vanilla peer receives score updates and removals.
use super::*;
use std::{collections::BTreeMap, io::Cursor};
use steel_core::{entity::SharedEntity, world::World};
use steel_registry::packets::play::{C_BOSS_EVENT, C_RESET_SCORE, C_SET_SCORE};
use steel_utils::{
    codec::VarInt,
    serial::{PrefixedRead, ReadFrom},
    text::DisplayResolutor,
};
use text_components::{TextComponent, format::Color};

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
                    next_entity_id(),
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
    assert_eq!(
        lines.len(),
        12,
        "three creatures and mana must coexist within the sidebar limit"
    );
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
    assert!(lines.iter().any(|line| contains(line, "Mana:")));
    for line in &lines {
        let text = displayed_text(line)?;
        let plain = text.to_plain(&DisplayResolutor);
        let expected = if plain.starts_with("Mana:") {
            Some(Color::Aqua)
        } else if plain.starts_with("HP:") {
            Some(Color::Green)
        } else if plain.starts_with("1. ") {
            Some(Color::Gold)
        } else if plain.starts_with('█') || plain.starts_with('░') {
            Some(Color::Blue)
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
    for expected in [
        "1. Oak",
        health.as_str(),
        "Super Ball · Deployed",
        "Mana: 50/200",
        "█████░░░░░░░░░░░░░░░",
    ] {
        assert!(
            lines.iter().any(|line| contains(line, expected)),
            "missing updated sidebar text: {expected}"
        );
    }
    assert!(!lines.iter().any(|line| contains(line, "1. Moss")));
    assert!(running.player.hud_conflicts().is_empty());
    let actor = Arc::clone(&running.player);
    common::on_tick(&running.server, move || {
        return_all(&actor)?;
        assert!(deployed.is_removed());
        Ok(())
    })
    .await
}
