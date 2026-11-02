use crate::entities::EntityPolicy;
const TEST_POLICY: EntityPolicy = EntityPolicy {
    minimum_health: 1.0,
    drop_loot: false,
};
use super::*;
use crate::host::test_support::{TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk};
use steel_core::player::player_data::PersistentPlayerData;
use steel_core::{bootstrap::init_globals, entity::next_entity_id};
use steel_registry::vanilla_entities;

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one native save and respawn scenario exercises dependent lifecycle transitions"
)]
fn store_deploy_recall_preserves_native_identity_health_and_dependency_fields() -> Result<(), String>
{
    init_globals();
    let fixture = fresh_test_world("owned_entity_roundtrip");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let cow = ENTITIES
        .create(
            &vanilla_entities::COW,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        )
        .ok_or("cow factory missing")?;
    cow.as_living_entity()
        .ok_or("cow should be living")?
        .set_health(7.0);
    world
        .try_add_entity(Arc::clone(&cow))
        .map_err(|e| e.to_string())?;
    let path = "test:entities/stored/0";
    StoredEntities::of(&player)?.store(&player, &cow, path, TEST_POLICY)?;
    assert!(cow.is_removed());
    assert!(
        StoredEntities::of(&player)?
            .store(&player, &cow, "test:entities/stored/1", TEST_POLICY)
            .is_err()
    );
    let nickname = DataPath::parse("test:entities/stored/0/nickname")?;
    player.data().set(&nickname, NbtTag::String("Moo".into()))?;
    let deployed = StoredEntities::of(&player)?.restore(
        &player,
        world,
        path,
        DVec3::new(10.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    assert_eq!(deployed.uuid(), cow.uuid());
    assert_eq!(
        deployed
            .as_living_entity()
            .ok_or("deployed cow should be living")?
            .get_health(),
        7.0
    );
    assert!(
        ChunkStorage::entity_tree_to_persistent(&deployed).is_none(),
        "owned entities must not also save in a chunk"
    );
    assert!(
        StoredEntities::of(&player)?
            .restore(
                &player,
                world,
                path,
                DVec3::new(12.5, 65.0, 8.5),
                TEST_POLICY
            )
            .is_err()
    );
    deployed
        .as_living_entity()
        .ok_or("deployed cow should be living")?
        .set_health(3.0);
    assert!(StoredEntities::of(&player)?.return_to_storage(&player, path)?);
    assert!(deployed.is_removed());
    let again = StoredEntities::of(&player)?.restore(
        &player,
        world,
        path,
        DVec3::new(12.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    assert_eq!(
        again
            .as_living_entity()
            .ok_or("deployed cow should be living")?
            .get_health(),
        3.0
    );
    assert_eq!(
        player.data().get(&nickname),
        Some(&NbtTag::String("Moo".into()))
    );
    let replacement = player.new_respawn_replacement(Arc::clone(world), false, false, true);
    assert_eq!(StoredEntities::of(&player)?.deployments().len(), 0);
    drop(player);
    assert!(
        !again.is_removed(),
        "dropping the old incarnation must not discard transferred entities"
    );
    assert_eq!(StoredEntities::of(&replacement)?.deployments().len(), 1);
    assert_eq!(
        replacement.data().get(&nickname),
        Some(&NbtTag::String("Moo".into()))
    );
    let saved = PersistentPlayerData::from_player(&replacement)?;
    assert_eq!(
        saved
            .custom_data
            .get(&DataPath::parse("test:entities/stored/0/health/current")?),
        Some(&NbtTag::Double(3.0))
    );
    StoredEntities::of(&replacement)?.recall_all(&replacement)?;
    assert!(again.is_removed());
    Ok(())
}

#[test]
fn corrupted_dependency_parent_refuses_a_player_snapshot_instead_of_losing_the_entity()
-> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("owned_entity_snapshot_error");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let cow = ENTITIES
        .create(
            &vanilla_entities::COW,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        )
        .ok_or("cow factory missing")?;
    world
        .try_add_entity(Arc::clone(&cow))
        .map_err(|e| e.to_string())?;
    StoredEntities::of(&player)?.store(&player, &cow, "test:entities/stored/0", TEST_POLICY)?;
    let deployed = StoredEntities::of(&player)?.restore(
        &player,
        world,
        "test:entities/stored/0",
        DVec3::new(10.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    player
        .data()
        .set(&DataPath::parse("test:entities/stored")?, NbtTag::Int(1))?;
    assert!(PersistentPlayerData::from_player(&player).is_err());
    assert!(
        !deployed.is_removed(),
        "failed persistence must leave the live entity recoverable"
    );
    Ok(())
}

#[test]
fn native_snapshot_restores_attributes_absorption_and_hidden_effects_without_reapplying_them()
-> Result<(), String> {
    use steel_core::entity::{MobEffectInstance, attribute::AttributeModifier};
    use steel_registry::{
        attribute::AttributeModifierOperation, vanilla_attributes, vanilla_mob_effects,
    };
    init_globals();
    let fixture = fresh_test_world("owned_entity_living_state");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let cow = ENTITIES
        .create(
            &vanilla_entities::COW,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        )
        .ok_or("cow factory missing")?;
    let living = cow.as_living_entity().ok_or("cow should be living")?;
    living.attributes().lock().set_modifier(
        vanilla_attributes::MAX_HEALTH,
        AttributeModifier {
            id: steel_utils::Identifier::new_static("entities_test", "health"),
            amount: 14.0,
            operation: AttributeModifierOperation::AddValue,
        },
        true,
    );
    living.add_mob_effect(MobEffectInstance::with_duration(
        vanilla_mob_effects::SPEED,
        600,
        0,
    ));
    living.add_mob_effect(MobEffectInstance::with_duration(
        vanilla_mob_effects::SPEED,
        100,
        2,
    ));
    living.add_mob_effect(MobEffectInstance::with_duration(
        vanilla_mob_effects::ABSORPTION,
        300,
        1,
    ));
    living.set_absorption_amount(5.0);
    living.set_health(17.0);
    let speed = living
        .attributes()
        .lock()
        .get_value(vanilla_attributes::MOVEMENT_SPEED);
    let effects = living.living_base().active_mob_effects();
    world
        .try_add_entity(Arc::clone(&cow))
        .map_err(|e| e.to_string())?;
    let path = "test:entities/stored/0";
    StoredEntities::of(&player)?.store(&player, &cow, path, TEST_POLICY)?;
    let deployed = StoredEntities::of(&player)?.restore(
        &player,
        world,
        path,
        DVec3::new(10.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    let loaded = deployed
        .as_living_entity()
        .ok_or("deployed cow should be living")?;
    assert_eq!(loaded.get_max_health(), 24.0);
    assert_eq!(loaded.get_health(), 17.0);
    assert_eq!(
        loaded.get_absorption_amount(),
        5.0,
        "loading must not grant fresh absorption hearts"
    );
    assert_eq!(
        loaded
            .attributes()
            .lock()
            .get_value(vanilla_attributes::MOVEMENT_SPEED),
        speed,
        "effect modifiers must survive exactly once"
    );
    for effect in effects {
        assert_eq!(
            loaded.living_base().mob_effect(effect.effect()),
            Some(effect)
        );
    }
    StoredEntities::of(&player)?.recall_all(&player)?;
    Ok(())
}

#[test]
fn replacement_conversion_after_respawn_keeps_one_owned_entity_and_recalls_the_new_type()
-> Result<(), String> {
    use steel_core::entity::{ConversionParams, convert_mob};
    init_globals();
    let fixture = fresh_test_world("owned_entity_conversion");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let player = TestPlayerBuilder::new(Arc::clone(world), "Trainer", next_entity_id()).build();
    let mooshroom = ENTITIES
        .create(
            &vanilla_entities::MOOSHROOM,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        )
        .ok_or("mooshroom factory missing")?;
    world
        .try_add_entity(Arc::clone(&mooshroom))
        .map_err(|e| e.to_string())?;
    let path = "test:entities/stored/0";
    StoredEntities::of(&player)?.store(&player, &mooshroom, path, TEST_POLICY)?;
    let deployed = StoredEntities::of(&player)?.restore(
        &player,
        world,
        path,
        DVec3::new(10.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    let replacement = player.new_respawn_replacement(Arc::clone(world), false, false, true);
    drop(player);
    let converted = convert_mob(
        deployed.as_mob().ok_or("mooshroom must be a mob")?,
        &vanilla_entities::COW,
        ConversionParams::single(false, false),
        None,
    )
    .ok_or("mooshroom conversion failed")?;
    assert!(deployed.is_removed());
    assert_ne!(converted.uuid(), deployed.uuid());
    let deployments = StoredEntities::of(&replacement)?.deployments();
    assert_eq!(deployments.len(), 1);
    assert!(Arc::ptr_eq(&deployments[0].1, &converted));
    assert!(ChunkStorage::entity_tree_to_persistent(&converted).is_none());
    assert!(
        StoredEntities::of(&replacement)?
            .store(
                &replacement,
                &converted,
                "test:entities/stored/1",
                TEST_POLICY
            )
            .is_err()
    );
    StoredEntities::of(&replacement)?.return_to_storage(&replacement, path)?;
    assert!(converted.is_removed());
    let restored = StoredEntities::of(&replacement)?.restore(
        &replacement,
        world,
        path,
        DVec3::new(12.5, 65.0, 8.5),
        TEST_POLICY,
    )?;
    assert_eq!(restored.entity_type(), &vanilla_entities::COW);
    assert_eq!(restored.uuid(), converted.uuid());
    StoredEntities::of(&replacement)?.recall_all(&replacement)?;
    Ok(())
}

#[test]
fn nonliving_item_entity_roundtrips_without_a_health_compound() -> Result<(), String> {
    use steel_core::entity::entities::ItemEntity;
    use steel_registry::{item_stack::ItemStack, vanilla_items};
    use steel_utils::Downcast;
    init_globals();
    let fixture = fresh_test_world("stored_native_item");
    let world = &fixture.world;
    let player = TestPlayerBuilder::new(Arc::clone(world), "Owner", next_entity_id()).build();
    let original: SharedEntity = Arc::new(ItemEntity::with_item_and_velocity(
        &vanilla_entities::ITEM,
        next_entity_id(),
        DVec3::new(8.5, 65.0, 8.5),
        ItemStack::new(&vanilla_items::DIAMOND),
        DVec3::ZERO,
        Arc::downgrade(world),
    ));
    world
        .try_add_entity(Arc::clone(&original))
        .map_err(|e| e.to_string())?;
    assert!(original.as_living_entity().is_none());
    let storage = StoredEntities::of(&player)?;
    let path = "test:entities/item";
    storage.store(&player, &original, path, EntityPolicy::default())?;
    assert!(
        player
            .data()
            .get(&DataPath::parse(&format!("{path}/health"))?)
            .is_none()
    );
    let restored = storage.restore(
        &player,
        world,
        path,
        DVec3::new(10.5, 65.0, 8.5),
        EntityPolicy::default(),
    )?;
    assert_eq!(restored.uuid(), original.uuid());
    assert_eq!(
        restored
            .downcast_ref::<ItemEntity>()
            .ok_or("item implementation missing")?
            .get_item()
            .item(),
        &*vanilla_items::DIAMOND
    );
    let released = storage.release(&player, world, path, DVec3::ZERO, EntityPolicy::default())?;
    assert!(Arc::ptr_eq(&released, &restored));
    assert!(ChunkStorage::entity_tree_to_persistent(&released).is_some());
    Ok(())
}
