//! Builds a registry mob registration from an extension's definition.

use steel_registry::{
    Registry,
    client_content::{EntityRegistration, MobCarrier, MobStyle as NativeStyle},
    equipment::EquipmentSlot,
};
use steel_utils::Identifier;
use text_components::TextComponent;

use crate::content::{EntityDefinition, MobStyle};

fn key(text: &str) -> Result<Identifier, String> {
    text.parse::<Identifier>().map_err(ToString::to_string)
}

pub(crate) fn register_entity(
    registry: &mut Registry,
    definition: EntityDefinition,
) -> Result<(), String> {
    let carrier_key = key(definition.carrier.as_str())?;
    let carrier = MobCarrier::from_key(&carrier_key).ok_or_else(|| {
        format!("{carrier_key} cannot carry a mob; use slime, magma_cube, zombie, or husk")
    })?;
    let mut equipment = Vec::with_capacity(definition.equipment.len());
    for piece in &definition.equipment {
        let slot = EquipmentSlot::by_name(piece.slot.as_str())
            .filter(|slot| !matches!(slot, EquipmentSlot::Body | EquipmentSlot::Saddle))
            .ok_or_else(|| format!("unknown equipment slot {}", piece.slot.as_str()))?;
        equipment.push((slot, super::appearance_stack(registry, &piece.appearance)?));
    }
    let attributes = definition
        .attributes
        .iter()
        .map(|attribute| Ok((key(attribute.key.as_str())?, attribute.value)))
        .collect::<Result<Vec<_>, String>>()?;
    registry.register_client_entity(EntityRegistration {
        key: key(definition.key.as_str())?,
        carrier,
        style: match definition.style {
            MobStyle::Cube => NativeStyle::Cube {
                size: definition.cube_size,
            },
            MobStyle::Melee => NativeStyle::Melee,
        },
        name: TextComponent::from(definition.name.as_str().to_owned()),
        name_visible: definition.name_visible,
        attributes,
        equipment,
        fire_immune: definition.fire_immune,
    })?;
    Ok(())
}
