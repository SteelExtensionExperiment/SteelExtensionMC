//! Typed content construction on the host side of the native boundary.

mod biome;
mod definition;
mod entity;
mod fluid;
pub(crate) mod resource_pack;
pub(crate) use biome::register_biome;
pub(crate) use entity::register_entity;
pub(crate) use fluid::register_fluid;
#[cfg(test)]
mod tests;

use std::num::NonZeroU32;
use std::sync::{Arc, OnceLock};

use glam::{Quat, Vec3};
use steel_core::behavior::BlockBehaviorRegistry;
use steel_registry::{
    Registry, RegistryExt,
    blocks::BlockRef,
    client_content::{
        self, BedModel, BedRegistration, BlockDisplayTransform, BlockItemDisplay, PlacingItem,
    },
    data_components::{
        DataComponentMap, DataComponentPatch,
        vanilla_components::{ITEM_MODEL, ITEM_NAME, MAX_STACK_SIZE, PROFILE},
    },
    game_profile::ProfileProperty,
    item_stack::ItemStack,
    items::{Item, ItemRef},
    resolvable_profile::{PartialProfile, PlayerSkinPatch, ResolvableProfile},
    vanilla_items,
};
use steel_utils::{Identifier, UuidExt};
use text_components::TextComponent;
use uuid::Uuid;

use super::{HookRegistration, Registration, lifecycle::ExtensionState};
use crate::{
    ApiResult, BlockState,
    block_behavior::{DynBlockBehavior, OriginalBlockBehavior, adapter::Adapter},
    content::{
        BedDefinition, BlockDefinition, BlockItemDefinition, ContentRegistry, HeadProfile,
        ItemAppearance, ItemDefinition, ItemDisplay, SlabMaterial,
    },
};
use stabby::{string::String as ApiString, vec::Vec as ApiVec};

type BlockBehaviorFactory = extern "C" fn(
    crate::Registry<'_>,
    BlockState,
    OriginalBlockBehavior,
) -> ApiResult<DynBlockBehavior>;

pub(crate) struct ContentRegistration<'a> {
    pub registry: &'a mut Registry,
    pub config: &'a str,
    block_behaviors: &'a mut Vec<(BlockRef, BlockBehaviorFactory)>,
}

impl ContentRegistration<'_> {
    pub(crate) fn register_block(
        &mut self,
        factory: Option<BlockBehaviorFactory>,
        register: impl FnOnce(&mut Registry) -> Result<BlockRef, String>,
    ) -> Result<BlockState, String> {
        let block = register(self.registry)?;
        if let Some(factory) = factory {
            self.block_behaviors.push((block, factory));
        }
        Ok(BlockState {
            id: self.registry.blocks.get_default_state_id(block).0,
        })
    }
}

pub(crate) fn subscribe(
    hooks: &mut HookRegistration<'_>,
    callback: extern "C" fn(ContentRegistry<'_>) -> ApiResult<()>,
) -> Result<(), String> {
    hooks.extension.begin_registration();
    let extension = Arc::clone(hooks.extension);
    let result = client_content::on_registration(move |registry| {
        extension.run_registration("content", || {
            register_content(registry, &extension, callback).into()
        });
    });
    if result.is_err() {
        hooks.extension.cancel_registration();
    }
    result
}

fn register_content(
    registry: &mut Registry,
    extension: &Arc<ExtensionState>,
    callback: extern "C" fn(ContentRegistry<'_>) -> ApiResult<()>,
) -> Result<(), ApiString> {
    let mut block_behaviors = Vec::new();
    let mut registration = ContentRegistration {
        registry,
        config: &extension.config,
        block_behaviors: &mut block_behaviors,
    };
    callback(ContentRegistry::from_host(&mut registration)).match_owned(Ok, Err)?;
    if block_behaviors.is_empty() {
        return Ok(());
    }

    extension.begin_registration();
    let owner = Arc::clone(extension);
    let result = BlockBehaviorRegistry::on_content_registration(move |behaviors| {
        owner.run_registration("custom block behavior", || {
            install_block_behaviors(behaviors, &owner.config, block_behaviors)
                .map_err(ApiString::from)
                .into()
        });
    });
    if result.is_err() {
        extension.cancel_registration();
    }
    result.map_err(ApiString::from)
}

fn install_block_behaviors(
    behaviors: &mut BlockBehaviorRegistry,
    config: &str,
    registrations: Vec<(BlockRef, BlockBehaviorFactory)>,
) -> Result<(), String> {
    for (block, factory) in registrations {
        let original = OriginalBlockBehavior::capture(behaviors.get_behavior_shared(block));
        let state = BlockState {
            id: block.default_state().0,
        };
        let mut access = Registration { behaviors, config };
        let behavior = factory(crate::Registry::from_host(&mut access), state, original)
            .match_owned(Ok, Err)
            .map_err(|error| format!("{}: {error}", block.key))?;
        behaviors.set_behavior(block, Box::new(Adapter(behavior)));
    }
    Ok(())
}

pub(crate) fn register_item(
    registry: &mut Registry,
    definition: ItemDefinition,
) -> Result<(), String> {
    let (item, fallback) = prepare_item(registry, definition)?;
    registry.register_client_item(item, fallback)?;
    Ok(())
}

fn prepare_item(
    registry: &Registry,
    definition: ItemDefinition,
) -> Result<(Item, ItemRef), String> {
    if !(1..=99).contains(&definition.max_stack_size) {
        return Err("item maximum stack size must be between 1 and 99".into());
    }
    let key = definition
        .key
        .as_str()
        .parse::<Identifier>()
        .map_err(ToString::to_string)?;
    let (fallback, mut components) = appearance(registry, &definition.appearance)?;
    components.set(
        ITEM_NAME,
        Some(TextComponent::from(definition.name.as_str().to_owned())),
    );
    components.set(MAX_STACK_SIZE, Some(definition.max_stack_size));
    Ok((
        Item {
            key,
            components,
            block: None,
            craft_remainder: None,
            id: OnceLock::new(),
        },
        fallback,
    ))
}

pub(crate) fn register_block(
    registry: &mut Registry,
    definition: BlockDefinition,
) -> Result<BlockRef, String> {
    let definition = definition::prepare(registry, definition)?;
    registry.register_block_definition(definition, None)
}

pub(crate) fn register_block_item(
    registry: &mut Registry,
    definition: BlockItemDefinition,
) -> Result<BlockRef, String> {
    let (block, placing) = prepare_block_item(registry, definition)?;
    registry.register_block_definition(block, Some(placing))
}

pub(crate) fn register_storage_block(
    registry: &mut Registry,
    definition: BlockItemDefinition,
    max_slots: u32,
) -> Result<BlockRef, String> {
    let slots = NonZeroU32::new(max_slots)
        .filter(|slots| i32::try_from(slots.get()).is_ok())
        .ok_or("storage slot count must fit in a positive NBT integer")?;
    let key = definition
        .block
        .key
        .as_str()
        .parse::<Identifier>()
        .map_err(str::to_owned)?;
    if registry.block_entity_types.by_key(&key).is_some() {
        return Err(format!("block entity type {key} is already registered"));
    }
    let block = register_block_item(registry, definition)?;
    registry.block_entity_types.register_storage(block, slots);
    Ok(block)
}

fn prepare_block_item(
    registry: &Registry,
    definition: BlockItemDefinition,
) -> Result<(client_content::BlockDefinition, PlacingItem), String> {
    let block = definition::prepare(registry, definition.block)?;
    let (item, fallback) = prepare_item(registry, definition.item)?;
    Ok((
        block,
        PlacingItem {
            item,
            fallback,
            drop_self: definition.drop_self,
        },
    ))
}

pub(crate) fn slab_materials(registry: &Registry) -> ApiVec<SlabMaterial> {
    registry
        .blocks
        .iter()
        .filter_map(|(_, block)| {
            if !registry.client_content.is_vanilla_block(block)
                || registry
                    .blocks
                    .state_id_from_block_defaulted_properties(
                        block,
                        [("type", "double"), ("waterlogged", "false")],
                    )
                    .is_none()
            {
                return None;
            }
            let item = registry.items.by_block(block);
            if item == &*vanilla_items::AIR {
                return None;
            }
            Some(SlabMaterial {
                block: block.key.to_string().into(),
                item: item.key.to_string().into(),
            })
        })
        .collect()
}

fn appearance(
    registry: &Registry,
    appearance: &ItemAppearance,
) -> Result<(ItemRef, DataComponentMap), String> {
    let key = appearance
        .item
        .as_str()
        .parse::<Identifier>()
        .map_err(ToString::to_string)?;
    let item = registry
        .items
        .by_key(&key)
        .ok_or_else(|| format!("unknown client item {key}"))?;
    if !registry.client_content.is_vanilla_item(item) || item == &*vanilla_items::AIR {
        return Err("the client appearance must use a nonempty vanilla item".into());
    }
    let mut components = DataComponentMap::common_item_components();
    components.set(ITEM_MODEL, item.components.get(ITEM_MODEL));
    if let Some(model) = appearance.model.as_ref() {
        components.set(
            ITEM_MODEL,
            Some(
                model
                    .as_str()
                    .parse::<Identifier>()
                    .map_err(ToString::to_string)?,
            ),
        );
    }
    if let Some(profile) = appearance.profile.as_ref() {
        if item != &*vanilla_items::PLAYER_HEAD {
            return Err("a head profile requires minecraft:player_head as its client item".into());
        }
        components.set(PROFILE, Some(head_profile(profile)?));
    }
    Ok((item, components))
}

fn head_profile(profile: &HeadProfile) -> Result<ResolvableProfile, String> {
    let id = Uuid::from_int_array(&profile.id).ok_or_else(|| "invalid profile UUID".to_owned())?;
    let property =
        ProfileProperty::new("textures".into(), profile.texture.as_str().to_owned(), None)
            .map_err(|error| error.to_string())?;
    let profile =
        PartialProfile::new(None, Some(id), vec![property]).map_err(|error| error.to_string())?;
    Ok(ResolvableProfile::static_partial(
        profile,
        PlayerSkinPatch::default(),
    ))
}

/// One stack showing an appearance: a patch with its model and skin over the vanilla item.
pub(crate) fn appearance_stack(
    registry: &Registry,
    appearance_definition: &ItemAppearance,
) -> Result<ItemStack, String> {
    let (item, components) = appearance(registry, appearance_definition)?;
    let mut patch = DataComponentPatch::new();
    if let Some(model) = components.get(ITEM_MODEL) {
        patch.set(ITEM_MODEL, model);
    }
    if let Some(profile) = components.get(PROFILE) {
        patch.set(PROFILE, profile);
    }
    Ok(ItemStack::with_count_and_patch(item, 1, patch))
}

fn prepare_displays(
    registry: &Registry,
    displays: &[ItemDisplay],
) -> Result<Vec<BlockItemDisplay>, String> {
    displays
        .iter()
        .map(|display| prepare_display(registry, display))
        .collect()
}

pub(crate) fn prepare_display(
    registry: &Registry,
    display: &ItemDisplay,
) -> Result<BlockItemDisplay, String> {
    Ok(BlockItemDisplay {
        item: appearance_stack(registry, &display.appearance)?,
        translation: Vec3::from_array(display.translation),
        scale: Vec3::from_array(display.scale),
        rotation: Quat::from_array(display.rotation),
        pack_transform: display
            .pack_transform
            .as_ref()
            .map(|transform| BlockDisplayTransform {
                translation: Vec3::from_array(transform.translation),
                scale: Vec3::from_array(transform.scale),
                rotation: Quat::from_array(transform.rotation),
            }),
    })
}

pub(crate) fn register_bed(
    registry: &mut Registry,
    definition: BedDefinition,
) -> Result<BlockState, String> {
    let key = definition
        .key
        .as_str()
        .parse::<Identifier>()
        .map_err(ToString::to_string)?;
    let template_key = definition
        .template
        .as_str()
        .parse::<Identifier>()
        .map_err(ToString::to_string)?;
    let template = registry
        .blocks
        .by_key(&template_key)
        .ok_or("unknown bed template")?;
    let (item, item_fallback) = prepare_item(registry, definition.item)?;
    let models = definition
        .models
        .iter()
        .map(|model| {
            Ok(BedModel {
                properties: model
                    .properties
                    .iter()
                    .map(|property| {
                        (
                            property.name.as_str().to_owned(),
                            property.value.as_str().to_owned(),
                        )
                    })
                    .collect(),
                displays: prepare_displays(registry, model.displays.as_slice())?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let block = registry.register_client_bed(BedRegistration {
        key,
        template,
        models,
        item,
        item_fallback,
    })?;
    Ok(BlockState {
        id: registry.blocks.get_default_state_id(block).0,
    })
}
