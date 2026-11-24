use super::{tests::finish, *};
use crate::{
    bootstrap::init_globals,
    config::ResolvedDomainConfig,
    permission::PermissionSubjectIndex,
    server::{WorldEntry, test_server_with_worlds_and_config},
    test_support::{
        TestPlayerBuilder, fresh_test_world, insert_ready_full_chunk, test_runtime_config,
    },
};
use crate::{
    entity::{Entity, LivingEntity},
    player::connection::NetworkConnection,
};
use std::slice::from_ref;
use steel_registry::vanilla_blocks;
use steel_utils::{ChunkPos, types::UpdateFlags};
use tokio::runtime::Builder;

#[test]
fn retiring_worlds_refuse_every_entry() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("retiring_home");
    let home = &fixture.world;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let domain = ResolvedDomainConfig {
            name: home.domain().to_owned(),
            default_world: home.key.clone(),
            worlds: vec![home.key.clone()],
        };
        let server = test_server_with_worlds_and_config(
            domain.name.clone(),
            from_ref(&domain),
            from_ref(home),
            PermissionSubjectIndex::new(),
            test_runtime_config(16),
            None,
        )
        .await?;
        let template_key = Identifier::new_static("retiring_test", "template");
        server.register_ready_template(super::clone_tests::chest_template(
            template_key.clone(),
            Identifier::from_steel("empty"),
        ));
        let key = Identifier::new(home.domain().to_owned(), "the_nether".to_owned());
        let id = server.clone_world(
            &template_key,
            key.clone(),
            DomainTarget::Existing,
            CloneOverrides {
                spawn: Some(BlockPos::new(0, 80, 0)),
                ..CloneOverrides::default()
            },
        )?;
        finish(&server, id).await?;
        let clone = server.worlds.get(&key).ok_or("clone not published")?;

        clone.begin_retiring();

        let player = TestPlayerBuilder::new(Arc::clone(home), "Steve", 1).build();
        let error = server
            .queue_player_world_selection(player, &clone, WorldEntry::Usual)
            .expect_err("world selection must refuse a retiring world");
        assert!(error.contains("being removed"), "{error}");
        assert!(server.worlds.resolve_nether_portal_target(home).is_none());
        assert!(
            server
                .resolve_saved_world(&key.to_string(), home.domain(), None, "Steve")
                .is_none()
        );
        Ok(())
    })
}

/// Runs jobs and world changes until `id` completes.
async fn drive(server: &Arc<Server>, id: WorldOperationId) -> Result<Identifier, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    for tick in 1.. {
        for world in server.worlds.values() {
            world.chunk_map.advance_scheduling();
        }
        server.process_world_changes(tick, true);
        server.process_domain_switches();
        server.process_player_joins();
        server.tick_jobs(tick, true);
        if let Some(result) = server.take_world_operation(id)? {
            return result;
        }
        if std::time::Instant::now() >= deadline {
            let players: Vec<_> = server
                .get_players()
                .iter()
                .map(|p| {
                    (
                        p.gameprofile.name().to_owned(),
                        p.get_world().key.to_string(),
                        p.is_world_change_pending(),
                        p.connection.closed(),
                        p.get_health(),
                    )
                })
                .collect();
            let admissions: Vec<_> = server
                .player_admissions
                .lock()
                .values()
                .map(|s| format!("{s:?}"))
                .collect();
            return Err(format!(
                "world operation did not finish: players {players:?} admissions {admissions:?} jobs {}",
                server.jobs.is_empty()
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    Err("tick identity exhausted".into())
}

async fn test_server(home: &Arc<World>) -> Result<Arc<Server>, String> {
    let domain = ResolvedDomainConfig {
        name: home.domain().to_owned(),
        default_world: home.key.clone(),
        worlds: vec![home.key.clone()],
    };
    let server = test_server_with_worlds_and_config(
        domain.name.clone(),
        from_ref(&domain),
        from_ref(home),
        PermissionSubjectIndex::new(),
        test_runtime_config(16),
        None,
    )
    .await?;
    server.register_ready_template(super::clone_tests::chest_template(
        Identifier::new_static("removal_test", "template"),
        Identifier::from_steel("empty"),
    ));
    Ok(server)
}

async fn clone_into(
    server: &Arc<Server>,
    key: &Identifier,
    target: DomainTarget,
) -> Result<Arc<World>, String> {
    let id = server.clone_world(
        &Identifier::new_static("removal_test", "template"),
        key.clone(),
        target,
        CloneOverrides {
            spawn: Some(BlockPos::new(0, 80, 0)),
            ..CloneOverrides::default()
        },
    )?;
    drive(server, id).await?;
    server
        .worlds
        .get(key)
        .ok_or_else(|| "clone not published".to_owned())
}

#[test]
fn removal_is_refused_for_configured_pending_shared_clock_and_closing_worlds() -> Result<(), String>
{
    init_globals();
    let fixture = fresh_test_world("removal_refusals_home");
    let home = &fixture.world;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let server = test_server(home).await?;
        assert!(!server.world_is_removable(&home.key));
        assert!(
            server
                .remove_world(&home.key)
                .expect_err("worlds.toml worlds stay")
                .contains("worlds.toml")
        );

        let template = Identifier::new_static("removal_test", "template");
        let pending = Identifier::new_static("rm_pending", "arena");
        let id = server.clone_world(
            &template,
            pending.clone(),
            DomainTarget::NewTemporary,
            CloneOverrides {
                spawn: Some(BlockPos::new(0, 80, 0)),
                ..CloneOverrides::default()
            },
        )?;
        assert!(
            server
                .remove_world(&pending)
                .expect_err("a world being created cannot be removed")
                .contains("being created")
        );
        drive(&server, id).await?;

        let sibling = server.create_temporary_world(TemporaryWorldRequest {
            key: Identifier::new_static("rm_pending", "sibling"),
            primary: false,
            generator: Identifier::from_steel("empty"),
            config: toml::from_str("dimension_type='minecraft:overworld'")
                .map_err(|e| e.to_string())?,
            seed: 0,
            spawn: BlockPos::new(0, 65, 0),
            default_gamemode: GameType::Adventure,
            difficulty: Difficulty::Peaceful,
        })?;
        drive(&server, sibling).await?;
        assert!(
            server
                .remove_world(&pending)
                .expect_err("the clock owner has a derived sibling")
                .contains("clock")
        );

        let destroy = server.destroy_temporary_domain("rm_pending")?;
        assert!(
            server
                .remove_world(&Identifier::new_static("rm_pending", "sibling"))
                .expect_err("closing domains refuse removal")
                .contains("closing")
        );
        drive(&server, destroy).await?;
        Ok(())
    })
}

#[test]
fn players_move_to_the_domain_default_then_the_server_default() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("removal_relocate_home");
    let home = &fixture.world;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let server = test_server(home).await?;
        let position = glam::DVec3::new(0.5, 80.0, 0.5);
        for z in -3..=3 {
            for x in -3..=3 {
                insert_ready_full_chunk(home, ChunkPos::new(x, z));
            }
        }
        for y in [64, 65] {
            assert!(home.set_block(
                BlockPos::new(0, y, 0),
                vanilla_blocks::STONE.default_state(),
                UpdateFlags::UPDATE_ALL,
            ));
        }

        let first_key = Identifier::new_static("rm_temp", "first");
        let second_key = Identifier::new_static("rm_temp", "second");
        let first = clone_into(&server, &first_key, DomainTarget::NewTemporary).await?;
        for y in [79, 80] {
            assert!(first.set_block(
                BlockPos::new(0, y, 0),
                vanilla_blocks::STONE.default_state(),
                UpdateFlags::UPDATE_ALL,
            ));
        }
        let second = clone_into(&server, &second_key, DomainTarget::Existing).await?;
        let bots = [
            server.spawn_bot(&second_key, "Rider", position)?,
            server.spawn_bot(&second_key, "Passenger", position)?,
        ];
        let id = server.remove_world(&second_key)?;
        assert!(!server.world_is_removable(&second_key));
        assert!(
            server
                .remove_world(&second_key)
                .expect_err("a world already being removed")
                .contains("already being removed")
        );
        assert_eq!(drive(&server, id).await?, second_key);
        assert!(server.worlds.get(&second_key).is_none());
        for bot in &bots {
            assert_eq!(bot.get_world().key, first.key, "domain default world");
        }

        let third_key = Identifier::new_static("rm_temp", "third");
        let third = clone_into(&server, &third_key, DomainTarget::Existing).await?;
        let id = server.remove_world(&first_key)?;
        assert_eq!(drive(&server, id).await?, first_key);
        let rider = Arc::clone(&bots[0]);
        for bot in &bots {
            assert_eq!(
                bot.get_world().key,
                home.key,
                "no domain default: server default"
            );
        }
        drop(second);

        assert!(server.worlds.has_domain("rm_temp"));
        assert!(server.worlds.default_world("rm_temp").is_none());
        assert!(
            server
                .queue_domain_switch(Arc::clone(&rider), "rm_temp".into())
                .is_err(),
            "a domain without a default world refuses switch_domain"
        );
        server.queue_player_world_selection(Arc::clone(&rider), &third, WorldEntry::Usual)?;
        let (respawn_world, _) = server.respawn_world_and_data_for_domain("rm_temp")?;
        assert_eq!(respawn_world.key, home.key);
        let error = server
            .clone_world(
                &Identifier::new_static("removal_test", "template"),
                Identifier::new_static("rm_temp", "fourth"),
                DomainTarget::Existing,
                CloneOverrides::default(),
            )
            .expect_err("a temporary domain without a default world takes no new worlds");
        assert!(error.contains("no default world"), "{error}");
        Ok(())
    })
}
