use crate::entities::EntityPolicy;
const TEST_POLICY: EntityPolicy = EntityPolicy {
    minimum_health: 1.0,
    drop_loot: false,
};
use super::*;
use crate::host::test_support::{TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk};
use steel_core::{
    bootstrap::init_globals,
    entity::{damage::DamageSource, next_entity_id},
};
use steel_registry::{
    entity_data::EntityPose, entity_type::EntityTypeRef, vanilla_damage_types, vanilla_entities,
};

#[test]
fn owned_mobs_keep_one_hp_and_can_heal_recall_and_release() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("entity_health_floor");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let path = "test:entities/stored/0";
    for kind in [
        &vanilla_entities::SHEEP,
        &vanilla_entities::SQUID,
        &vanilla_entities::BAT,
        &vanilla_entities::COW,
        &vanilla_entities::MOOSHROOM,
        &vanilla_entities::PIG,
        &vanilla_entities::CHICKEN,
        &vanilla_entities::ENDERMITE,
    ] {
        let original = mob(world, kind)?;
        StoredEntities::of(&player)?.store(&player, &original, path, TEST_POLICY)?;
        let owned = StoredEntities::of(&player)?.restore(
            &player,
            world,
            path,
            DVec3::new(10.5, 65.0, 8.5),
            TEST_POLICY,
        )?;
        // Squid can apply environmental damage while returning false from hurtServer.
        owned.hurt(
            world,
            &DamageSource::environment(&vanilla_damage_types::GENERIC_KILL),
            f32::MAX,
        );
        let living = owned.as_living_entity().ok_or("mob is not living")?;
        assert_eq!(
            living.get_health(),
            1.0,
            "the caller-selected floor must prevent native death processing"
        );
        assert!(!living.living_base().is_death_processed());
        assert_ne!(owned.pose(), EntityPose::Dying);
        for health in [0.0, -5.0, 0.5] {
            living.set_health(health);
            assert_eq!(
                living.get_health(),
                1.0,
                "direct health writes must also honor the floor"
            );
        }
        living.heal(2.0);
        assert_eq!(living.get_health(), 3.0);
        living.set_health(1.0);
        StoredEntities::of(&player)?.return_to_storage(&player, path)?;
        assert_eq!(
            player
                .data()
                .get(&DataPath::parse(&format!("{path}/health/current"))?)
                .and_then(NbtTag::double),
            Some(1.0)
        );
        let released = StoredEntities::of(&player)?.release(
            &player,
            world,
            path,
            DVec3::new(10.5, 65.0, 8.5),
            TEST_POLICY,
        )?;
        assert_eq!(
            released
                .as_living_entity()
                .ok_or("mob is not living")?
                .get_health(),
            1.0
        );
        released.kill(world);
        assert_eq!(
            released
                .as_living_entity()
                .ok_or("mob is not living")?
                .get_health(),
            0.0,
            "released mobs must return to vanilla mortality"
        );
        released.set_removed(RemovalReason::Discarded);
    }
    Ok(())
}

fn mob(world: &Arc<World>, kind: EntityTypeRef) -> Result<SharedEntity, String> {
    let entity = ENTITIES
        .create(
            kind,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        )
        .ok_or("mob factory missing")?;
    world
        .try_add_entity(Arc::clone(&entity))
        .map_err(|error| error.to_string())?;
    Ok(entity)
}

#[test]
fn stored_zero_hp_snapshot_recovers_to_one_hp_without_changing_identity_or_dependencies()
-> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("stored_inactive_entity");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let original = mob(world, &vanilla_entities::SHEEP)?;
    original.kill(world);
    let living = original.as_living_entity().ok_or("mob is not living")?;
    assert_eq!(living.get_health(), 0.0);
    living.living_base().increment_death_time();
    // Serialize the native dead entity to reproduce snapshots saved by the old ownership rules.
    let snapshot = record::snapshot(&original)?;
    original.set_removed(RemovalReason::Discarded);
    let path = "test:entities/stored/0";
    player
        .data()
        .set(&DataPath::parse(path)?, NbtTag::Compound(snapshot))?;
    let note = DataPath::parse(&format!("{path}/nickname"))?;
    player.data().set(&note, NbtTag::String("Kept".into()))?;
    let deployed = StoredEntities::of(&player)?.restore(
        &player,
        world,
        path,
        DVec3::new(10.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    let living = deployed.as_living_entity().ok_or("mob is not living")?;
    assert_eq!(deployed.uuid(), original.uuid());
    assert_eq!(living.get_health(), 1.0);
    assert_eq!(living.living_base().death_time(), 0);
    assert!(!living.living_base().is_death_processed());
    assert_ne!(deployed.pose(), EntityPose::Dying);
    assert_eq!(
        player.data().get(&note),
        Some(&NbtTag::String("Kept".into()))
    );
    living.heal(2.0);
    assert_eq!(living.get_health(), 3.0);
    StoredEntities::of(&player)?.return_to_storage(&player, path)?;
    let released = StoredEntities::of(&player)?.release(
        &player,
        world,
        path,
        DVec3::new(10.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    assert_eq!(
        released
            .as_living_entity()
            .ok_or("mob is not living")?
            .get_health(),
        3.0
    );
    assert!(ChunkStorage::entity_tree_to_persistent(&released).is_some());
    Ok(())
}

#[test]
fn release_recovers_a_stored_zero_hp_entity_without_needing_deployment_first() -> Result<(), String>
{
    init_globals();
    let fixture = fresh_test_world("release_stored_inactive_entity");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let original = mob(world, &vanilla_entities::BAT)?;
    original.kill(world);
    let snapshot = record::snapshot(&original)?;
    original.set_removed(RemovalReason::Discarded);
    let path = "test:entities/stored/0";
    player
        .data()
        .set(&DataPath::parse(path)?, NbtTag::Compound(snapshot))?;
    let released = StoredEntities::of(&player)?.release(
        &player,
        world,
        path,
        DVec3::new(10.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    assert_eq!(released.uuid(), original.uuid());
    assert_eq!(
        released
            .as_living_entity()
            .ok_or("mob is not living")?
            .get_health(),
        1.0
    );
    assert!(player.data().get(&DataPath::parse(path)?).is_none());
    assert!(ChunkStorage::entity_tree_to_persistent(&released).is_some());
    assert!(!released.is_removed());
    Ok(())
}

#[test]
fn store_applies_the_floor_only_if_the_store_succeeds() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("store_health_floor_rollback");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let first = mob(world, &vanilla_entities::SHEEP)?;
    let occupied = "test:entities/stored/0";
    StoredEntities::of(&player)?.store(&player, &first, occupied, TEST_POLICY)?;
    let candidate = mob(world, &vanilla_entities::BAT)?;
    let living = candidate.as_living_entity().ok_or("mob is not living")?;
    living.set_health(0.5);
    let before = player.data().snapshot();
    assert!(
        StoredEntities::of(&player)?
            .store(&player, &candidate, occupied, TEST_POLICY)
            .is_err()
    );
    assert_eq!(
        living.get_health(),
        0.5,
        "failed store must not heal an unowned mob"
    );
    assert!(candidate.base().attachment().is_none());
    assert_eq!(player.data().snapshot(), before);
    StoredEntities::of(&player)?.store(
        &player,
        &candidate,
        "test:entities/stored/1",
        TEST_POLICY,
    )?;
    assert_eq!(living.get_health(), 1.0);
    assert_eq!(
        player
            .data()
            .get(&DataPath::parse("test:entities/stored/1/health/current")?)
            .and_then(NbtTag::double),
        Some(1.0)
    );
    Ok(())
}

#[test]
fn caller_selected_floor_and_default_native_death_are_independent() -> Result<(), String> {
    init_globals();
    for minimum in [0.0, 2.5] {
        let fixture = fresh_test_world("explicit_entity_health_policy");
        let world = &fixture.world;
        let player = TestPlayerBuilder::new(Arc::clone(world), "Owner", next_entity_id()).build();
        let original = mob(world, &vanilla_entities::SHEEP)?;
        let policy = EntityPolicy {
            minimum_health: minimum,
            ..EntityPolicy::default()
        };
        let storage = StoredEntities::of(&player)?;
        let path = "test:entities/one";
        storage.store(&player, &original, path, policy)?;
        let restored =
            storage.restore(&player, world, path, DVec3::new(10.5, 65.0, 8.5), policy)?;
        restored.kill(world);
        let living = restored
            .as_living_entity()
            .ok_or("sheep should be living")?;
        assert_eq!(living.get_health(), minimum);
        assert_eq!(living.living_base().is_death_processed(), minimum == 0.0);
    }
    Ok(())
}
