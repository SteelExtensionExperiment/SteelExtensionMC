use super::*;
use text_components::{Modifier, format::Color};
use toml::de::Error as TomlError;

#[test]
fn owner_configuration_combines_independent_data_and_replaces_default_sections()
-> Result<(), String> {
    let config: toml::Table = toml::from_str(include_str!(
        "../../../../../doc/extensions/examples/combined-hud.toml"
    ))
    .map_err(|e| e.to_string())?;
    let policy: HudPolicy = config["server"]["hud"]
        .clone()
        .try_into()
        .map_err(|e: TomlError| e.to_string())?;
    let mut data = CustomData::default();
    for (path, value) in [
        ("magic:state/mana/current", NbtTag::Double(25.0)),
        ("magic:state/mana/maximum", NbtTag::Double(100.0)),
        (
            "creatures:state/party/0/label",
            NbtTag::String("Moss".into()),
        ),
        (
            "creatures:state/party/0/health/current",
            NbtTag::Double(3.0),
        ),
        (
            "creatures:state/party/0/health/maximum",
            NbtTag::Double(12.0),
        ),
    ] {
        data.set(&DataPath::parse(path)?, value)?;
    }
    let mut hud = PlayerHud::default();
    for owner in ["creatures-example:default", "magic-example:default"] {
        hud.composition.sources.insert(
            catalog::SourceKey(owner.into()),
            section("Default title", &["Default line"]),
        );
    }
    hud.composition.server_tab = Some((
        TextComponent::plain("Default server tab"),
        TextComponent::plain("Default footer"),
    ));
    let (_, errors) = hud.reconcile_for_test(&data, &policy);
    assert!(errors.is_empty(), "{errors:?}");
    let sidebar = hud.sidebar.as_ref().ok_or("configured sidebar missing")?;
    assert_eq!(sidebar.title.to_plain(&DisplayResolutor), "");
    let plain: Vec<_> = sidebar
        .lines
        .iter()
        .map(|line| line.to_plain(&DisplayResolutor))
        .collect();
    assert_eq!(plain, ["Moss: 3/12 HP"]);
    assert_eq!(sidebar.lines[0].format.color, Some(Color::Gold));
    let (header, footer) = hud.tab_list.as_ref().ok_or("configured tab missing")?;
    assert_eq!(header.to_plain(&DisplayResolutor), "Moss · Mana 25/100");
    assert_eq!(footer.to_plain(&DisplayResolutor), "█████░░░░░░░░░░░░░░░");

    let packed = hud.composition.render(&data, &policy, true);
    let packed_sidebar = packed.sidebar?.ok_or("configured packed sidebar missing")?;
    let packed_meter = TextComponent::plain("█████░░░░░░░░░░░░░░░")
        .font("magic:hud/mana")
        .color(Color::White);
    assert_eq!(
        packed_sidebar.lines[0].to_plain(&DisplayResolutor),
        "Moss: 3/12 HP █░░░░"
    );
    assert_eq!(packed.tab?.1, packed_meter);

    data.set(
        &DataPath::parse("magic:state/mana/current")?,
        NbtTag::Double(50.0),
    )?;
    let (packets, errors) = hud.reconcile_for_test(&data, &policy);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(
        !packets.is_empty(),
        "data changes must refresh the configured HUD"
    );
    let sidebar = hud.sidebar.as_ref().ok_or("updated sidebar missing")?;
    assert_eq!(
        sidebar.lines[0].to_plain(&DisplayResolutor),
        "Moss: 3/12 HP"
    );
    let (header, footer) = hud.tab_list.as_ref().ok_or("updated tab missing")?;
    assert_eq!(header.to_plain(&DisplayResolutor), "Moss · Mana 50/100");
    assert_eq!(footer.to_plain(&DisplayResolutor), "██████████░░░░░░░░░░");

    data.remove(&DataPath::parse("creatures:state/party/0")?);
    let (_, errors) = hud.reconcile_for_test(&data, &policy);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(hud.sidebar.is_none());
    assert_eq!(
        hud.tab_list
            .as_ref()
            .ok_or("tab missing")?
            .0
            .to_plain(&DisplayResolutor),
        ""
    );
    Ok(())
}
