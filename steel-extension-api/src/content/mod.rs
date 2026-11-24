//! Startup registration of independent server items and blocks for vanilla clients.

mod bindings;
mod biome;
mod definition;
mod entity;
mod fluid;
pub use biome::{BiomeDefinition, BiomeParticle};
pub use entity::{EntityDefinition, MobAttribute, MobEquipment, MobStyle};
mod shapes;
pub use bindings::api;
pub use bindings::api::ContentRegistry;
pub use definition::{BlockDefinition, BlockProperties, HorizontalPlacement, SlabMaterial};
pub use fluid::{FluidDefinition, FluidTint, RegisteredFluid};
pub use shapes::{BlockShapes, ShapeBox, VoxelShape};

use stabby::{option::Option as ApiOption, string::String as ApiString, vec::Vec as ApiVec};

/// A static player-head profile; `texture` is the base64 textures property.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct HeadProfile {
    /// UUID in vanilla's four-signed-integer NBT representation.
    pub id: [i32; 4],
    /// Base64-encoded `textures` property supplied by the extension.
    pub texture: ApiString,
}

/// Vanilla item appearance. A profile is meaningful only for a player head.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct ItemAppearance {
    /// Vanilla registry key, for example `minecraft:player_head`.
    pub item: ApiString,
    /// Optional model key. Models contributed to Steel's pack use the vanilla
    /// item appearance on connections that have not loaded the pack.
    pub model: ApiOption<ApiString>,
    /// Static skin profile when the appearance is a player head.
    pub profile: ApiOption<HeadProfile>,
}

impl ItemAppearance {
    /// Uses a vanilla item as the client appearance when no model is selected.
    #[must_use]
    pub fn new(item: &str) -> Self {
        Self {
            item: item.into(),
            model: None.into(),
            profile: None.into(),
        }
    }

    /// Selects an item model by key.
    #[must_use]
    pub fn with_model(mut self, model: &str) -> Self {
        self.model = Some(model.into()).into();
        self
    }
}

/// An independent server item. Use `BlockItemDefinition` for placement behavior.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct ItemDefinition {
    /// Owner-namespaced server identity, retained in saved inventories.
    pub key: ApiString,
    /// Literal item name displayed to the client.
    pub name: ApiString,
    /// The vanilla item and optional skin used on the client.
    pub appearance: ItemAppearance,
    /// Maximum stack size, from 1 through 99.
    pub max_stack_size: i32,
}

/// A display transform using block-relative translation and XYZW rotation.
#[stabby::stabby]
#[derive(Debug, Clone, Copy)]
pub struct DisplayTransform {
    /// XYZ translation from the block minimum corner.
    pub translation: [f32; 3],
    /// Positive XYZ scale.
    pub scale: [f32; 3],
    /// Unit quaternion in XYZW order.
    pub rotation: [f32; 4],
}

/// One client-only item display. Coordinates are relative to the placed block's
/// minimum corner; item display context is vanilla `none`.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct ItemDisplay {
    /// Item rendered by this display.
    pub appearance: ItemAppearance,
    /// XYZ translation from the block's minimum corner, in blocks.
    pub translation: [f32; 3],
    /// Positive XYZ model scale.
    pub scale: [f32; 3],
    /// Left rotation quaternion in XYZW order.
    pub rotation: [f32; 4],
    /// Optional transform for the model supplied by the combined pack.
    pub pack_transform: ApiOption<DisplayTransform>,
}

impl ItemDisplay {
    /// Centers a full-block model over a block using a vanilla block item as fallback.
    #[must_use]
    pub fn cube(appearance: ItemAppearance) -> Self {
        Self {
            appearance,
            translation: [0.5; 3],
            // Extend just beyond the physical carrier to avoid intersecting faces.
            scale: [1.002; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            pack_transform: None.into(),
        }
    }
}

/// One assembled model and the world positions of the construction parts it covers.
#[stabby::stabby]
pub struct MultiblockDisplay {
    /// Model drawn at the storage owner's position.
    pub model: ItemDisplay,
    /// Positions whose construction displays the model covers.
    pub parts: ApiVec<crate::BlockPos>,
}

/// One named vanilla state property, such as `type=top` on a slab.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct BlockProperty {
    /// Vanilla property name, such as `type` or `facing`.
    pub name: ApiString,
    /// Vanilla property value, such as `top` or `east`.
    pub value: ApiString,
}

/// A vanilla block and property overrides over its registered default state.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct BlockStateDefinition {
    /// Registered vanilla block key.
    pub block: ApiString,
    /// Named overrides; unspecified properties retain their vanilla defaults.
    pub properties: ApiVec<BlockProperty>,
}

impl From<&str> for BlockStateDefinition {
    fn from(block: &str) -> Self {
        Self {
            block: block.into(),
            properties: ApiVec::new(),
        }
    }
}

/// Registers a block and its placing item together, resolving their association
/// and optional self-drop without forward references to unregistered content.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct BlockItemDefinition {
    /// Block identity, physical carrier, and visual model.
    pub block: BlockDefinition,
    /// Placing item identity and inventory appearance.
    pub item: ItemDefinition,
    /// Drops exactly one placing item when loot is requested. This fixed policy
    /// does not inherit the carrier's loot table or explosion decay.
    pub drop_self: bool,
}

/// Built-in Minecraft heads that fit a cube display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VanillaHead {
    /// Default player head.
    Player,
    /// Skeleton skull.
    Skeleton,
    /// Wither skeleton skull.
    WitherSkeleton,
    /// Zombie head.
    Zombie,
    /// Creeper head.
    Creeper,
}

/// How a textured cube appears to a client that has not loaded Steel's pack.
#[stabby::stabby]
#[repr(C, u8)]
#[derive(Debug, Clone, Default)]
pub enum TextureFallback {
    /// Use the placing item's vanilla base.
    #[default]
    Vanilla,
    /// Use an existing Minecraft head in inventory and placement.
    Head {
        /// Cubical vanilla head key or published Minecraft skin URL or hash.
        skin: ApiString,
    },
}

impl TextureFallback {
    /// Selects a built-in head available to vanilla clients without a download.
    #[must_use]
    pub fn vanilla_head(head: VanillaHead) -> Self {
        let item = match head {
            VanillaHead::Player => "minecraft:player_head",
            VanillaHead::Skeleton => "minecraft:skeleton_skull",
            VanillaHead::WitherSkeleton => "minecraft:wither_skeleton_skull",
            VanillaHead::Zombie => "minecraft:zombie_head",
            VanillaHead::Creeper => "minecraft:creeper_head",
        };
        Self::Head { skin: item.into() }
    }
}

impl BlockItemDefinition {
    /// Uses one full-cube model for the placing item and the placed block.
    /// Steel uses the placing item's vanilla base for both when the client has
    /// not loaded the pack. Use a vanilla full-cube block item as the base.
    #[must_use]
    pub fn with_cube_model(mut self, model: &str) -> Self {
        self.item.appearance = self.item.appearance.with_model(model);
        self.block.displays = [ItemDisplay::cube(self.item.appearance.clone())]
            .into_iter()
            .collect();
        self
    }
}

/// A model selected by named bed-state properties. Selectors must not overlap.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct BlockModel {
    /// Partial selector, for example `part=head,facing=north`.
    pub properties: ApiVec<BlockProperty>,
    /// Additional model parts around the vanilla bed.
    pub displays: ApiVec<ItemDisplay>,
}

/// A real two-part bed with ordinary sleeping, occupancy, and respawn behavior.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct BedDefinition {
    /// Extension-owned block key.
    pub key: ApiString,
    /// Ordinary vanilla colored bed whose state family and appearance are used.
    /// Straw beds have different rules and are not accepted here.
    pub template: ApiString,
    /// Placing item; mining either half returns one of these items.
    pub item: ItemDefinition,
    /// Optional model additions for particular parts and orientations.
    pub models: ApiVec<BlockModel>,
}
