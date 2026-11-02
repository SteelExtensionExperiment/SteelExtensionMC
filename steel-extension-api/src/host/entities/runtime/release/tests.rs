use crate::entities::EntityPolicy;
const TEST_POLICY: EntityPolicy = EntityPolicy {
    minimum_health: 1.0,
    drop_loot: false,
};
use super::*;
use crate::host::test_support::{TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk};
use simdnbt::owned::NbtTag;
use steel_core::{
    bootstrap::init_globals,
    chunk_saver::ChunkStorage,
    entity::{ENTITIES, Entity, next_entity_id},
};
use steel_registry::{vanilla_blocks, vanilla_entities};
use steel_utils::{BlockPos, types::UpdateFlags};

#[test]
fn stored_release_preserves_native_state_frees_slot_and_enables_chunk_saving() -> Result<(), String>
{
    init_globals();
    let fixture = fresh_test_world("stored_entity_release");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let entity = ENTITIES
        .create(
            &vanilla_entities::SHEEP,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        )
        .ok_or("sheep factory missing")?;
    entity
        .as_living_entity()
        .ok_or("sheep is not living")?
        .set_health(3.0);
    world
        .try_add_entity(Arc::clone(&entity))
        .map_err(|error| error.to_string())?;
    let path = "test:entities/stored/0";
    StoredEntities::of(&player)?.store(&player, &entity, path, TEST_POLICY)?;
    let sibling = DataPath::parse("other:state/value")?;
    player.data().set(&sibling, NbtTag::Int(7))?;
    let released = StoredEntities::of(&player)?.release(
        &player,
        world,
        path,
        DVec3::new(10.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    assert_eq!(released.uuid(), entity.uuid());
    assert_eq!(
        released
            .as_living_entity()
            .ok_or("sheep is not living")?
            .get_health(),
        3.0
    );
    assert!(player.data().get(&DataPath::parse(path)?).is_none());
    assert_eq!(player.data().get(&sibling), Some(&NbtTag::Int(7)));
    assert!(StoredEntities::of(&player)?.deployments().is_empty());
    assert!(ChunkStorage::entity_tree_to_persistent(&released).is_some());
    assert!(
        StoredEntities::of(&player)?
            .release(
                &player,
                world,
                path,
                DVec3::new(12.5, 65.0, 8.5),
                TEST_POLICY
            )
            .is_err()
    );
    StoredEntities::of(&player)?.recall_all(&player)?;
    assert!(!released.is_removed());
    assert!(Arc::ptr_eq(
        &world
            .get_entity_by_uuid(&released.uuid())
            .ok_or("released sheep missing")?,
        &released
    ));
    StoredEntities::of(&player)?.store(&player, &released, path, TEST_POLICY)?;
    assert!(
        released.is_removed(),
        "released mobs must be capturable again"
    );
    Ok(())
}

#[test]
fn blocked_release_retains_the_saved_entity() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("blocked_entity_release");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let entity = ENTITIES
        .create(
            &vanilla_entities::SHEEP,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        )
        .ok_or("sheep factory missing")?;
    world
        .try_add_entity(Arc::clone(&entity))
        .map_err(|error| error.to_string())?;
    let path = "test:entities/stored/0";
    StoredEntities::of(&player)?.store(&player, &entity, path, TEST_POLICY)?;
    let before = player.data().snapshot();
    world.set_block(
        BlockPos::new(10, 65, 8),
        vanilla_blocks::STONE.default_state(),
        UpdateFlags::UPDATE_NONE,
    );
    assert!(
        StoredEntities::of(&player)?
            .release(
                &player,
                world,
                path,
                DVec3::new(10.5, 65.0, 8.5),
                TEST_POLICY
            )
            .is_err()
    );
    assert_eq!(player.data().snapshot(), before);
    assert!(StoredEntities::of(&player)?.deployments().is_empty());
    assert!(world.get_entity_by_uuid(&entity.uuid()).is_none());
    Ok(())
}

#[test]
fn active_release_keeps_the_same_world_entity() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("deployed_entity_release");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let entity = ENTITIES
        .create(
            &vanilla_entities::SHEEP,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        )
        .ok_or("sheep factory missing")?;
    world
        .try_add_entity(Arc::clone(&entity))
        .map_err(|error| error.to_string())?;
    let path = "test:entities/stored/0";
    StoredEntities::of(&player)?.store(&player, &entity, path, TEST_POLICY)?;
    let deployed = StoredEntities::of(&player)?.restore(
        &player,
        world,
        path,
        DVec3::new(10.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    let released = StoredEntities::of(&player)?.release(
        &player,
        world,
        path,
        DVec3::new(f64::NAN, 0.0, 0.0),
        TEST_POLICY,
    )?;
    assert!(Arc::ptr_eq(&deployed, &released));
    assert_eq!(released.position(), DVec3::new(10.5, 65.0, 8.5));
    assert!(ChunkStorage::entity_tree_to_persistent(&released).is_some());
    StoredEntities::of(&player)?.recall_all(&player)?;
    assert!(!released.is_removed());
    Ok(())
}
