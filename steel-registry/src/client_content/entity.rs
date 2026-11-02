//! Server-only entity types projected onto a vanilla "carrier" entity type.
//!
//! Clients only know vanilla entity types, so a registered type is sent as its
//! carrier: the spawn packet names the carrier, and the synced data uses the
//! carrier's layout. Saves, commands, and gameplay keep the registered type.

use steel_utils::Identifier;
use text_components::TextComponent;

use crate::{
    Registry, RegistryExt,
    entity_type::{EntityType, EntityTypeRef},
    equipment::EquipmentSlot,
    item_stack::ItemStack,
    vanilla_entities,
};

/// How the host drives a registered mob. Each style is a vanilla behavior the host
/// implements; extensions add gameplay through hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobStyle {
    /// Vanilla slime movement: hops toward a target or a random direction, and
    /// damages players by touching them (requires a cube carrier).
    Cube {
        /// Cube size; vanilla slimes are 1, 2, or 4.
        size: i32,
    },
    /// A walking melee monster that targets players, like a zombie (requires a
    /// humanoid carrier).
    Melee,
}

/// Vanilla entity types that can carry a registered mob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobCarrier {
    Slime,
    MagmaCube,
    Zombie,
    Husk,
}

impl MobCarrier {
    /// The carrier named by a vanilla entity type key.
    #[must_use]
    pub fn from_key(key: &Identifier) -> Option<Self> {
        let carriers: [(EntityTypeRef, Self); 4] = [
            (&vanilla_entities::SLIME, Self::Slime),
            (&vanilla_entities::MAGMA_CUBE, Self::MagmaCube),
            (&vanilla_entities::ZOMBIE, Self::Zombie),
            (&vanilla_entities::HUSK, Self::Husk),
        ];
        carriers
            .into_iter()
            .find(|(entity_type, _)| &entity_type.key == key)
            .map(|(_, carrier)| carrier)
    }

    /// The vanilla entity type clients see.
    #[must_use]
    pub fn entity_type(self) -> EntityTypeRef {
        match self {
            Self::Slime => &vanilla_entities::SLIME,
            Self::MagmaCube => &vanilla_entities::MAGMA_CUBE,
            Self::Zombie => &vanilla_entities::ZOMBIE,
            Self::Husk => &vanilla_entities::HUSK,
        }
    }

    /// Whether the carrier is a cube mob (slime-like).
    #[must_use]
    pub const fn is_cube(self) -> bool {
        matches!(self, Self::Slime | Self::MagmaCube)
    }
}

/// One registered mob type.
#[derive(Debug)]
pub struct ClientEntity {
    pub entity_type: EntityTypeRef,
    pub carrier: MobCarrier,
    pub style: MobStyle,
    /// Shown in death messages and, when `name_visible`, above the mob.
    pub name: TextComponent,
    pub name_visible: bool,
    /// Visible equipment; never dropped.
    pub equipment: Vec<(EquipmentSlot, ItemStack)>,
}

/// Everything needed to register one mob type.
pub struct EntityRegistration {
    pub key: Identifier,
    pub carrier: MobCarrier,
    pub style: MobStyle,
    pub name: TextComponent,
    pub name_visible: bool,
    /// Attribute base values replacing the carrier's, by attribute key.
    pub attributes: Vec<(Identifier, f64)>,
    pub equipment: Vec<(EquipmentSlot, ItemStack)>,
    pub fire_immune: bool,
}

impl Registry {
    /// Registers a mob type carried by a vanilla entity type.
    pub fn register_client_entity(
        &mut self,
        definition: EntityRegistration,
    ) -> Result<&'static ClientEntity, String> {
        self.validate_content_key(&definition.key)?;
        if self.entity_types.by_key(&definition.key).is_some() {
            return Err(format!(
                "entity type {} is already registered",
                definition.key
            ));
        }
        match definition.style {
            MobStyle::Cube { size }
                if !definition.carrier.is_cube() || !(1..=8).contains(&size) =>
            {
                return Err(
                    "cube mobs need a slime or magma cube carrier and a size from 1 to 8".into(),
                );
            }
            MobStyle::Melee if definition.carrier.is_cube() => {
                return Err("melee mobs need a zombie or husk carrier".into());
            }
            _ => {}
        }
        for (key, value) in &definition.attributes {
            // Entity default attributes name vanilla attributes by path.
            if key.namespace != Identifier::VANILLA_NAMESPACE
                || self.attributes.by_key(key).is_none()
            {
                return Err(format!("unknown attribute {key}"));
            }
            if !value.is_finite() {
                return Err(format!("attribute {key} must be finite"));
            }
        }
        let carrier = definition.carrier.entity_type();
        let attributes = merge_attributes(carrier.default_attributes, &definition.attributes);
        let entity_type: EntityTypeRef = Box::leak(Box::new(EntityType {
            key: definition.key,
            client_tracking_range: carrier.client_tracking_range,
            update_interval: carrier.update_interval,
            track_deltas: carrier.track_deltas,
            dimensions: carrier.dimensions,
            fixed: carrier.fixed,
            mob_category: carrier.mob_category,
            fire_immune: definition.fire_immune,
            summonable: true,
            allowed_in_peaceful: carrier.allowed_in_peaceful,
            can_spawn_far_from_player: carrier.can_spawn_far_from_player,
            can_serialize: true,
            only_op_can_set_nbt: carrier.only_op_can_set_nbt,
            is_abstract_boat: false,
            is_abstract_minecart: false,
            is_projectile: false,
            is_abstract_arrow: false,
            is_abstract_horse: false,
            is_abstract_nautilus: false,
            flags: carrier.flags,
            default_attributes: attributes,
        }));
        self.entity_types.register(entity_type);
        let entry: &'static ClientEntity = Box::leak(Box::new(ClientEntity {
            entity_type,
            carrier: definition.carrier,
            style: definition.style,
            name: definition.name,
            name_visible: definition.name_visible,
            equipment: definition.equipment,
        }));
        self.client_content.entities.push(entry);
        Ok(entry)
    }
}

/// The carrier's base values with the overrides applied, as a process-lifetime slice.
fn merge_attributes(
    carrier: &'static [(&'static str, f64)],
    overrides: &[(Identifier, f64)],
) -> &'static [(&'static str, f64)] {
    let mut merged: Vec<(&'static str, f64)> = carrier.to_vec();
    for (key, value) in overrides {
        let path = key.path.as_ref();
        if let Some(slot) = merged.iter_mut().find(|(existing, _)| *existing == path) {
            slot.1 = *value;
        } else {
            merged.push((Box::leak(path.to_owned().into_boxed_str()), *value));
        }
    }
    Box::leak(merged.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ghoul() -> EntityRegistration {
        EntityRegistration {
            key: Identifier::new_static("entity_test", "ghoul"),
            carrier: MobCarrier::Husk,
            style: MobStyle::Melee,
            name: TextComponent::from("Ghoul".to_owned()),
            name_visible: false,
            attributes: vec![(Identifier::vanilla_static("max_health"), 40.0)],
            equipment: Vec::new(),
            fire_immune: true,
        }
    }

    #[test]
    fn registered_mob_copies_its_carrier_and_overrides_attributes() -> Result<(), String> {
        let mut registry = Registry::new_vanilla();
        let entry = registry.register_client_entity(ghoul())?;
        let husk = &vanilla_entities::HUSK;
        assert_eq!(entry.entity_type.dimensions, husk.dimensions);
        assert_eq!(entry.entity_type.mob_category, husk.mob_category);
        assert!(entry.entity_type.fire_immune);
        let health = entry
            .entity_type
            .default_attributes
            .iter()
            .find(|(key, _)| *key == "max_health")
            .map(|(_, value)| *value);
        assert_eq!(health, Some(40.0));
        assert!(
            registry
                .entity_types
                .by_key(&entry.entity_type.key)
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn invalid_mobs_are_rejected() {
        let mut registry = Registry::new_vanilla();
        let cases: [fn(&mut EntityRegistration); 5] = [
            |d| d.key = Identifier::vanilla_static("ghoul"),
            |d| d.style = MobStyle::Cube { size: 2 },
            |d| {
                d.carrier = MobCarrier::Slime;
            },
            |d| d.attributes = vec![(Identifier::vanilla_static("not_an_attribute"), 1.0)],
            |d| d.attributes = vec![(Identifier::vanilla_static("max_health"), f64::NAN)],
        ];
        for mutate in cases {
            let mut definition = ghoul();
            mutate(&mut definition);
            assert!(registry.register_client_entity(definition).is_err());
        }
        let mut cube = ghoul();
        cube.carrier = MobCarrier::Slime;
        cube.style = MobStyle::Cube { size: 9 };
        assert!(registry.register_client_entity(cube).is_err());
        assert!(registry.client_content.entities.is_empty());
    }
}
