use std::collections::BTreeMap;

use simdnbt::owned::NbtTag;
use steel_protocol::packets::game::{BossBarColor, BossBarOverlay};
use steel_utils::text::DisplayResolutor;
use text_components::TextComponent;
use uuid::Uuid;

use super::{
    composition::{Contributions, OwnedBar},
    *,
};
use crate::entity::custom_data::{CustomData, DataPath};

#[path = "tests/owner_layout.rs"]
mod owner_layout;

fn section(title: &str, lines: &[&str]) -> composition::Contributions {
    Contributions {
        sidebar: Some(SidebarSection {
            title: title.into(),
            lines: lines.iter().map(|s| (*s).into()).collect(),
        }),
        ..Default::default()
    }
}

#[test]
fn independent_sections_survive_removal_and_overflow_retains_the_last_valid_view() {
    let mut hud = PlayerHud::default();
    let data = CustomData::default();
    let mut policy = HudPolicy::default();
    let magic = catalog::SourceKey("magic:default".into());
    let creatures = catalog::SourceKey("creatures:default".into());
    hud.composition
        .sources
        .insert(magic.clone(), section("Magic", &["Mana"]));
    hud.composition
        .sources
        .insert(creatures, section("Creatures", &["Cow", "Pig"]));
    let (_, errors) = hud.reconcile_for_test(&data, &policy);
    assert_eq!(errors, Vec::<String>::new());
    let displayed = || hud.sidebar.as_ref().map(|s| s.lines.len());
    assert_eq!(displayed(), Some(3));
    hud.composition
        .sources
        .insert(magic.clone(), section("Magic", &["overflow"; 15]));
    hud.rendered_revision = None;
    let (packets, errors) = hud.reconcile_for_test(&data, &policy);
    assert!(packets.is_empty());
    assert_eq!(hud.sidebar.as_ref().map(|s| s.lines.len()), Some(3));
    assert!(errors.join("").contains("creatures:default, magic:default"));
    policy.hidden.insert(magic.0.clone());
    hud.rendered_revision = None;
    let (_, errors) = hud.reconcile_for_test(&data, &policy);
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(hud.sidebar.as_ref().map(|s| s.lines.len()), Some(2));
    hud.composition.sources.remove(&magic);
    hud.rendered_revision = None;
    assert!(hud.reconcile_for_test(&data, &policy).0.is_empty());
}

#[test]
fn untitled_sections_keep_their_lines_without_title_separators() -> Result<(), String> {
    let mut hud = PlayerHud::default();
    let data = CustomData::default();
    let mut policy = HudPolicy::default();
    let creatures = catalog::SourceKey("untitled-creatures:default".into());
    let magic = catalog::SourceKey("untitled-magic:default".into());
    hud.composition
        .sources
        .insert(creatures, section("", &["Cow"]));
    hud.composition
        .sources
        .insert(magic.clone(), section("", &["Mana"]));
    let (_, errors) = hud.reconcile_for_test(&data, &policy);
    assert!(errors.is_empty());
    let sidebar = hud
        .sidebar
        .as_ref()
        .ok_or("untitled sidebar must still exist")?;
    assert_eq!(sidebar.title.to_plain(&DisplayResolutor), "");
    assert_eq!(sidebar.lines.len(), 2);

    hud.composition
        .sources
        .insert(magic, section("Magic", &["Mana"]));
    hud.rendered_revision = None;
    let (_, errors) = hud.reconcile_for_test(&data, &policy);
    assert!(errors.is_empty());
    assert_eq!(
        hud.sidebar
            .as_ref()
            .ok_or("sidebar missing")?
            .title
            .to_plain(&DisplayResolutor),
        "Magic"
    );

    policy.sidebar_title = Some("Adventure".into());
    hud.rendered_revision = None;
    let (_, errors) = hud.reconcile_for_test(&data, &policy);
    assert!(errors.is_empty());
    assert_eq!(
        hud.sidebar
            .as_ref()
            .ok_or("sidebar missing")?
            .title
            .to_plain(&DisplayResolutor),
        "Adventure"
    );
    Ok(())
}

#[test]
fn alias_collisions_are_source_local_and_owner_templates_require_an_explicit_choice()
-> Result<(), String> {
    let first = catalog::register("hud-test-a", "default")?;
    let second = catalog::register("hud-test-b", "default")?;
    assert_eq!(first, catalog::register("hud-test-a", "default")?);
    catalog::bind(first, "shared_test_value", "alpha:state/value")?;
    catalog::bind(second, "shared_test_value", "beta:state/value")?;
    let mut data = CustomData::default();
    data.set(&DataPath::parse("alpha:state/value")?, NbtTag::Int(3))?;
    data.set(&DataPath::parse("beta:state/value")?, NbtTag::Int(7))?;
    let overrides = BTreeMap::new();
    let source = catalog::source(first)?;
    let context = expression::RenderContext {
        data: &data,
        owner: Some(&source.key),
        overrides: &overrides,
    };
    assert_eq!(
        context
            .text("${shared_test_value}")?
            .ok_or("missing data")?
            .to_plain(&DisplayResolutor),
        "3"
    );
    let context = expression::RenderContext {
        data: &data,
        owner: None,
        overrides: &overrides,
    };
    let error = context
        .text("${shared_test_value}")
        .err()
        .ok_or("ambiguous alias should fail")?;
    assert!(error.contains("hud-test-a:default"));
    assert!(error.contains("hud-test-b:default"));
    let overrides = BTreeMap::from([("shared_test_value".into(), "beta:state/value".into())]);
    let context = expression::RenderContext {
        data: &data,
        owner: None,
        overrides: &overrides,
    };
    assert_eq!(
        context
            .text("${shared_test_value}")?
            .ok_or("missing data")?
            .to_plain(&DisplayResolutor),
        "7"
    );
    Ok(())
}

#[test]
fn identical_local_bar_keys_remain_independent_and_data_drives_rich_text() -> Result<(), String> {
    let mut hud = PlayerHud::default();
    let mut data = CustomData::default();
    data.set(
        &DataPath::parse("magic:state/mana/current")?,
        NbtTag::Double(25.0),
    )?;
    data.set(
        &DataPath::parse("magic:state/mana/maximum")?,
        NbtTag::Double(100.0),
    )?;
    let policy = HudPolicy::default();
    let local = Uuid::from_u128(1);
    let source_a = catalog::SourceKey("magic:default".into());
    let source_b = catalog::SourceKey("creatures:default".into());
    let bar = BarContribution {
        title: "Mana ${magic:state/mana}".into(),
        gauge: Some("magic:state/mana".into()),
        bar: BossBar {
            title: TextComponent::plain(""),
            progress: 1.0,
            color: BossBarColor::Blue,
            overlay: BossBarOverlay::Progress,
            flags: 7,
        },
    };
    for (source, id) in [
        (source_a.clone(), Uuid::from_u128(2)),
        (source_b, Uuid::from_u128(3)),
    ] {
        hud.composition
            .sources
            .entry(source)
            .or_default()
            .bars
            .insert(
                local,
                OwnedBar {
                    id,
                    contribution: bar.clone(),
                },
            );
    }
    let (_, errors) = hud.reconcile_for_test(&data, &policy);
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(hud.boss_bars.len(), 2);
    assert!(
        hud.boss_bars
            .values()
            .all(|bar| (bar.progress - 0.25).abs() < f32::EPSILON)
    );
    hud.composition.sources.remove(&source_a);
    hud.rendered_revision = None;
    let (packets, errors) = hud.reconcile_for_test(&data, &policy);
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(packets.len(), 1);
    assert_eq!(hud.boss_bars.len(), 1);
    let aliases = BTreeMap::from([("player_mana".into(), "magic:state/mana".into())]);
    let context = expression::RenderContext {
        data: &data,
        owner: None,
        overrides: &aliases,
    };
    let text = context
        .text(r#"{"text":"${bar(player_mana, 4)} ${player_mana}","color":"blue"}"#)?
        .ok_or("missing data")?;
    assert_eq!(text.to_plain(&DisplayResolutor), "█░░░ 25");
    assert!(context.text("${bar(player_mana, 9999999)}").is_err());
    data.set(
        &DataPath::parse("magic:state/label")?,
        NbtTag::String("\",\"bold\":true".into()),
    )?;
    let context = expression::RenderContext {
        data: &data,
        owner: None,
        overrides: &aliases,
    };
    let text = context
        .text(r#"{"text":"${magic:state/label}"}"#)?
        .ok_or("missing label")?;
    assert_eq!(text.to_plain(&DisplayResolutor), "\",\"bold\":true");
    Ok(())
}
