//! Registered mob types, shown to clients as a vanilla "carrier" mob.
//!
//! The server keeps the type's own key (in saves, commands, and gameplay) while
//! clients render the carrier with the definition's name and equipment. The host
//! runs a vanilla behavior for the chosen style; `entity_behavior` hooks add the rest.

use stabby::{string::String as ApiString, vec::Vec as ApiVec};

use super::ItemAppearance;

/// How the host drives a registered mob.
#[stabby::stabby]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobStyle {
    /// Vanilla slime movement and contact damage; needs `minecraft:slime` or
    /// `minecraft:magma_cube`, and uses `cube_size`.
    Cube,
    /// Zombie-like walking melee attacker; needs `minecraft:zombie` or `minecraft:husk`.
    Melee,
}

/// A visible piece of equipment. It is never dropped.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct MobEquipment {
    /// `head`, `chest`, `legs`, `feet`, `mainhand`, or `offhand`.
    pub slot: ApiString,
    /// The item shown, such as a player head with a skin.
    pub appearance: ItemAppearance,
}

/// One attribute base value, such as `minecraft:max_health` = 40.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct MobAttribute {
    /// Vanilla attribute key.
    pub key: ApiString,
    /// Base value.
    pub value: f64,
}

/// A mob type.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct EntityDefinition {
    /// Entity type key, in the owner's namespace.
    pub key: ApiString,
    /// Vanilla entity type clients see: slime, magma cube, zombie, or husk.
    pub carrier: ApiString,
    /// Name in death messages and, when `name_visible`, above the mob.
    pub name: ApiString,
    /// Attribute base values replacing the carrier's.
    pub attributes: ApiVec<MobAttribute>,
    /// Visible equipment.
    pub equipment: ApiVec<MobEquipment>,
    /// Cube size for `MobStyle::Cube` (vanilla slimes are 1, 2, or 4); ignored otherwise.
    pub cube_size: i32,
    /// The vanilla behavior the host runs.
    pub style: MobStyle,
    /// Whether the name always shows above the mob.
    pub name_visible: bool,
    /// Whether fire and lava do not hurt it.
    pub fire_immune: bool,
}

impl EntityDefinition {
    /// A cube mob of `size` carried by `carrier`.
    #[must_use]
    pub fn cube(key: &str, carrier: &str, name: &str, size: i32) -> Self {
        Self::new(key, carrier, name, MobStyle::Cube, size)
    }

    /// A melee mob carried by `carrier`.
    #[must_use]
    pub fn melee(key: &str, carrier: &str, name: &str) -> Self {
        Self::new(key, carrier, name, MobStyle::Melee, 0)
    }

    fn new(key: &str, carrier: &str, name: &str, style: MobStyle, cube_size: i32) -> Self {
        Self {
            key: key.into(),
            carrier: carrier.into(),
            name: name.into(),
            attributes: ApiVec::new(),
            equipment: ApiVec::new(),
            cube_size,
            style,
            name_visible: false,
            fire_immune: false,
        }
    }

    /// Sets an attribute base value.
    #[must_use]
    pub fn attribute(mut self, key: &str, value: f64) -> Self {
        self.attributes.push(MobAttribute {
            key: key.into(),
            value,
        });
        self
    }

    /// Shows an item in an equipment slot.
    #[must_use]
    pub fn equip(mut self, slot: &str, appearance: ItemAppearance) -> Self {
        self.equipment.push(MobEquipment {
            slot: slot.into(),
            appearance,
        });
        self
    }
}
