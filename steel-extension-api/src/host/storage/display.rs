//! Registration and refresh of a storage block's optional vanilla visual.

use std::sync::{Arc, LazyLock};

use rustc_hash::FxHashMap;
use steel_core::{
    block_entity::{BlockEntity, BlockEntityTicker, StorageBlockEntity, StorageDisplay},
    world::World,
};
use steel_registry::{REGISTRY, Registry, RegistryExt as _, client_content::BlockDisplayTransform};
use steel_utils::{BlockPos, BlockStateId, Downcast as _, Identifier, locks::SyncRwLock};

use crate::content::{ItemDisplay, MultiblockDisplay};

type StorageDisplayResolver =
    extern "C" fn(crate::World<'_>, crate::BlockPos) -> stabby::option::Option<ItemDisplay>;

type MultiblockDisplayResolver =
    extern "C" fn(crate::World<'_>, crate::BlockPos) -> stabby::option::Option<MultiblockDisplay>;

#[derive(Clone, Copy)]
enum ResolveDisplay {
    Single(StorageDisplayResolver),
    Multiblock(MultiblockDisplayResolver),
}

#[derive(Clone, Copy)]
struct Resolver {
    radius: u8,
    resolve: ResolveDisplay,
}

static RESOLVERS: LazyLock<SyncRwLock<FxHashMap<Identifier, Resolver>>> =
    LazyLock::new(SyncRwLock::default);

pub(crate) fn register(
    registry: &Registry,
    block: &str,
    radius: u8,
    resolve: StorageDisplayResolver,
) -> Result<(), String> {
    let key = storage_key(registry, block)?;
    let mut resolvers = RESOLVERS.write();
    if resolvers.contains_key(&key) {
        return Err(format!("{key} already has a storage display resolver"));
    }
    resolvers.insert(
        key,
        Resolver {
            radius,
            resolve: ResolveDisplay::Single(resolve),
        },
    );
    Ok(())
}

pub(crate) fn register_multiblock(
    registry: &mut Registry,
    block: &str,
    radius: u8,
    parts: &[crate::BlockState],
    resolve: MultiblockDisplayResolver,
) -> Result<(), String> {
    let key = storage_key(registry, block)?;
    let mut resolvers = RESOLVERS.write();
    if resolvers.contains_key(&key) {
        return Err(format!("{key} already has a storage display resolver"));
    }
    if parts.is_empty() {
        return Err("multiblock display requires construction parts".into());
    }
    let states: Vec<_> = parts.iter().map(|state| BlockStateId(state.id)).collect();
    registry.register_construction_parts(&states)?;
    resolvers.insert(
        key,
        Resolver {
            radius,
            resolve: ResolveDisplay::Multiblock(resolve),
        },
    );
    Ok(())
}

fn storage_key(registry: &Registry, block: &str) -> Result<Identifier, String> {
    let key: Identifier = block.parse().map_err(str::to_owned)?;
    if !registry
        .blocks
        .by_key(&key)
        .is_some_and(|block| registry.block_entity_types.is_storage_block(block))
    {
        return Err(format!("{key} is not a registered storage block"));
    }
    Ok(key)
}

pub(crate) fn ticker(block: &str) -> Result<BlockEntityTicker, String> {
    let key: Identifier = block.parse().map_err(str::to_owned)?;
    let kind = REGISTRY
        .block_entity_types
        .by_key(&key)
        .ok_or("unknown storage block entity type")?;
    if !RESOLVERS.read().contains_key(&key) {
        return Err(format!("{key} has no storage display resolver"));
    }
    Ok(BlockEntityTicker::new(kind, refresh))
}

fn refresh(world: &Arc<World>, pos: BlockPos, _state: BlockStateId, entity: &dyn BlockEntity) {
    let Some(storage) = entity.downcast_ref::<StorageBlockEntity>() else {
        return;
    };
    let Some(resolver) = RESOLVERS.read().get(&entity.get_type().key).copied() else {
        return;
    };
    if !storage.appearance_needs_refresh(world, resolver.radius) {
        return;
    }
    let api_pos = crate::BlockPos {
        x: pos.x(),
        y: pos.y(),
        z: pos.z(),
    };
    let api_world = crate::World::from_host(world);
    let result = match resolver.resolve {
        ResolveDisplay::Single(resolve) => resolve(api_world, api_pos).match_owned(
            |model| prepare_display(&model, Box::default()).map(Some),
            || Ok(None),
        ),
        ResolveDisplay::Multiblock(resolve) => resolve(api_world, api_pos).match_owned(
            |assembly| {
                let parts =
                    construction_positions(pos, resolver.radius, assembly.parts.as_slice())?;
                prepare_display(&assembly.model, parts).map(Some)
            },
            || Ok(None),
        ),
    };
    match result {
        Ok(display) => storage.set_display(world, display),
        Err(error) => log::warn!("Storage display at {pos:?}: {error}"),
    }
}

fn construction_positions(
    owner: BlockPos,
    radius: u8,
    parts: &[crate::BlockPos],
) -> Result<Box<[BlockPos]>, String> {
    let radius = u32::from(radius);
    let mut positions = Vec::with_capacity(parts.len());
    for part in parts {
        if owner.x().abs_diff(part.x) > radius
            || owner.y().abs_diff(part.y) > radius
            || owner.z().abs_diff(part.z) > radius
        {
            return Err("multiblock display part is outside the resolver radius".into());
        }
        let part = BlockPos::new(part.x, part.y, part.z);
        if part != owner {
            positions.push(part);
        }
    }
    positions.sort_unstable_by_key(|pos| (pos.x(), pos.y(), pos.z()));
    positions.dedup();
    Ok(positions.into_boxed_slice())
}

fn prepare_display(model: &ItemDisplay, parts: Box<[BlockPos]>) -> Result<StorageDisplay, String> {
    let model = crate::host::content::prepare_display(&REGISTRY, model)?;
    let transform = BlockDisplayTransform {
        translation: model.translation,
        scale: model.scale,
        rotation: model.rotation,
    };
    if !transform.is_valid()
        || model
            .pack_transform
            .is_some_and(|transform| !transform.is_valid())
    {
        return Err("invalid storage display transform".to_owned());
    }
    Ok(StorageDisplay { model, parts })
}
