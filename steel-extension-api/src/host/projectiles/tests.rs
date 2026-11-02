use super::*;
use crate::host::test_support::{fresh_test_world, insert_ready_full_chunk};
use std::sync::atomic::AtomicUsize;
use steel_core::chunk_saver::ChunkStorage;
use steel_core::{
    behavior::init_behaviors,
    entity::{entities::SnowballEntity, next_entity_id},
};
use steel_registry::{init_vanilla_registry, vanilla_blocks};
use steel_utils::{BlockPos, ChunkPos, types::UpdateFlags};

struct Hits(Arc<AtomicUsize>);
impl ImpactHandler for Hits {
    fn on_hit(
        &self,
        projectile: &Arc<ExtensionProjectile>,
        _world: &Arc<World>,
        hit: &ProjectileHit,
    ) {
        assert!(projectile.is_removed());
        assert!(matches!(hit, ProjectileHit::Block { .. }));
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn native_collision_calls_the_handler_once_and_never_saves_callbacks() -> Result<(), String> {
    init_vanilla_registry();
    init_behaviors();
    let fixture = fresh_test_world("custom_projectile_collision");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    assert!(world.set_block(
        BlockPos::new(10, 65, 8),
        vanilla_blocks::STONE.default_state(),
        UpdateFlags::UPDATE_NONE
    ));
    let hits = Arc::new(AtomicUsize::new(0));
    let projectile = Arc::new(ExtensionProjectile::new(
        next_entity_id(),
        DVec3::new(8.5, 65.5, 8.5),
        Arc::downgrade(world),
        0.0,
        100,
        Box::new(Hits(Arc::clone(&hits))),
    )?);
    projectile.set_velocity(DVec3::X);
    world
        .try_add_entity(Arc::clone(&projectile) as SharedEntity)
        .map_err(|e| e.to_string())?;
    for _ in 0..5 {
        Arc::clone(&projectile).tick();
    }
    assert_eq!(hits.load(Ordering::Relaxed), 1);
    assert!(projectile.position().x <= 10.0);
    assert!(!projectile.is_persistent());
    assert!(!projectile.can_use_portal(true));
    let entity: SharedEntity = projectile;
    assert!(ChunkStorage::entity_tree_to_persistent(&entity).is_none());
    Ok(())
}

#[test]
fn lifetime_allows_the_last_tick_of_movement_without_a_false_impact() -> Result<(), String> {
    init_vanilla_registry();
    init_behaviors();
    let fixture = fresh_test_world("custom_projectile_lifetime");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let hits = Arc::new(AtomicUsize::new(0));
    let start = DVec3::new(8.5, 70.0, 8.5);
    let projectile = Arc::new(ExtensionProjectile::new(
        next_entity_id(),
        start,
        Arc::downgrade(world),
        0.0,
        1,
        Box::new(Hits(Arc::clone(&hits))),
    )?);
    projectile.set_velocity(DVec3::X);
    world
        .try_add_entity(Arc::clone(&projectile) as SharedEntity)
        .map_err(|e| e.to_string())?;
    Arc::clone(&projectile).tick();
    assert!(projectile.position().x > start.x);
    assert!(projectile.is_removed());
    assert_eq!(hits.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn snowball_client_predicts_the_same_path_between_updates() -> Result<(), String> {
    init_vanilla_registry();
    init_behaviors();
    for gravity in [0.0, 0.03] {
        let server_fixture = fresh_test_world("extension_projectile_server_prediction");
        let client_fixture = fresh_test_world("extension_projectile_client_prediction");
        let server = &server_fixture.world;
        let client = &client_fixture.world;
        insert_ready_full_chunk(server, ChunkPos::new(0, 0));
        insert_ready_full_chunk(client, ChunkPos::new(0, 0));
        let start = DVec3::new(8.5, 75.0, 8.5);
        let projectile = Arc::new(ExtensionProjectile::new(
            next_entity_id(),
            start,
            Arc::downgrade(server),
            gravity,
            100,
            Box::new(Hits(Arc::new(AtomicUsize::new(0)))),
        )?);
        projectile.set_velocity(DVec3::new(0.1, 0.05, 0.0));
        server
            .try_add_entity(Arc::clone(&projectile) as SharedEntity)
            .map_err(|error| error.to_string())?;
        // The vanilla client instantiates a snowball and assigns its synced metadata.
        let replica = Arc::new(SnowballEntity::new(
            &vanilla_entities::SNOWBALL,
            next_entity_id(),
            start,
            Arc::downgrade(client),
        ));
        replica.set_velocity(projectile.velocity());
        replica.set_no_gravity(
            projectile
                .synced_data()
                .ok_or("projectile metadata missing")?
                .is_no_gravity(),
        );
        client
            .try_add_entity(Arc::clone(&replica) as SharedEntity)
            .map_err(|error| error.to_string())?;
        for tick in 1..=vanilla_entities::SNOWBALL.update_interval {
            Arc::clone(&projectile).tick();
            Arc::clone(&replica).tick();
            assert!(
                projectile.position().distance(replica.position()) < 1.0e-10,
                "gravity={gravity}, tick={tick}: client predicts {:?}, server has {:?}",
                replica.position(),
                projectile.position(),
            );
            assert!(!projectile.is_removed());
            assert!(!replica.is_removed());
        }
    }
    Ok(())
}
