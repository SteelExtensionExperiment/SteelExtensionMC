use super::{tests::finish, *};
use crate::{
    block_entity::BLOCK_ENTITIES,
    bootstrap::init_globals,
    chunk::{
        Chunk,
        section::{ChunkSection, Sections},
        status::ChunkStatus,
    },
    chunk_saver::{CHUNK_COMPRESSION_LEVEL, ChunkStorage},
    config::ResolvedDomainConfig,
    level_data::WorldGenerationSettings,
    permission::PermissionSubjectIndex,
    server::test_server_with_worlds_and_config,
    test_support::{fresh_test_world, test_runtime_config},
    world::{
        LevelReader,
        template::{TemplateChunk, WorldTemplate},
    },
};
use std::{slice::from_ref, sync::Weak};
use steel_registry::{
    vanilla_block_entity_types, vanilla_blocks, vanilla_dimension_types::OVERWORLD,
};
use steel_utils::{ChunkPos, types::UpdateFlags};
use tokio::runtime::Builder;

const CHEST_POS: BlockPos = BlockPos::new(3, 64, 5);
const SECTION_HEIGHT: i32 = 16;

/// One saved chunk at (0, 0) holding a chest, as an overworld-height template.
fn chest_template(key: Identifier, generator: Identifier) -> Arc<WorldTemplate> {
    let section_count = (OVERWORLD.height / SECTION_HEIGHT) as usize;
    let chunk = Chunk::new(
        Sections::from_owned(
            (0..section_count)
                .map(|_| ChunkSection::new_empty())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        ),
        ChunkPos::new(0, 0),
        OVERWORLD.min_y,
        OVERWORLD.height,
        Weak::new(),
    );
    let state = vanilla_blocks::CHEST.default_state();
    chunk.set_block_state_for_generation(
        ChunkStatus::Features,
        CHEST_POS,
        state,
        UpdateFlags::UPDATE_NONE,
    );
    let entity = BLOCK_ENTITIES.create_or_unimplemented(
        &vanilla_block_entity_types::CHEST,
        Weak::new(),
        CHEST_POS,
        state,
    );
    assert!(chunk.set_block_entity(entity));
    let prepared = ChunkStorage::prepare_chunk_save(&chunk, ChunkStatus::Features, &[], true)
        .expect("forced save should prepare");
    let encoded = wincode::serialize(&prepared.persistent).expect("chunk should encode");
    let payload = zstd::encode_all(encoded.as_slice(), CHUNK_COMPRESSION_LEVEL)
        .expect("chunk should compress");
    let generation = WorldGenerationSettings::from_generator_config(
        generator,
        &toml::from_str("dimension_type='minecraft:overworld'").expect("config"),
        OVERWORLD.key.clone(),
        OVERWORLD.min_y,
        OVERWORLD.height,
    );
    Arc::new(WorldTemplate::from_chunks(
        key,
        generation,
        [(
            ChunkPos::new(0, 0),
            TemplateChunk {
                status: ChunkStatus::Features,
                payload: payload.into(),
            },
        )]
        .into_iter()
        .collect(),
    ))
}

#[test]
fn clones_read_the_template_independently_and_reject_duplicate_keys() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("clone_home");
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
        let template_key = Identifier::new_static("clone_test", "template");
        server.register_ready_template(chest_template(
            template_key.clone(),
            Identifier::from_steel("empty"),
        ));
        let key_a = Identifier::new_static("clone_a", "arena");
        let key_b = Identifier::new_static("clone_b", "arena");
        let overrides = CloneOverrides {
            spawn: Some(BlockPos::new(0, 80, 0)),
            ..CloneOverrides::default()
        };

        let id_a = server.clone_world(
            &template_key,
            key_a.clone(),
            DomainTarget::NewTemporary,
            overrides,
        )?;
        assert!(
            server
                .clone_world(
                    &template_key,
                    key_a.clone(),
                    DomainTarget::NewTemporary,
                    overrides
                )
                .is_err(),
            "a pending key is refused before preparation"
        );
        let id_b = server.clone_world(
            &template_key,
            key_b.clone(),
            DomainTarget::NewTemporary,
            overrides,
        )?;
        finish(&server, id_a).await?;
        finish(&server, id_b).await?;
        assert!(
            server
                .clone_world(
                    &template_key,
                    key_a.clone(),
                    DomainTarget::NewTemporary,
                    overrides
                )
                .is_err(),
            "a loaded key is refused"
        );

        let a = server.worlds.get(&key_a).ok_or("clone a not published")?;
        let b = server.worlds.get(&key_b).ok_or("clone b not published")?;
        for world in [&a, &b] {
            assert_eq!(world.game_time.ticks(), 0);
            assert_eq!(
                world.get_block_state(CHEST_POS),
                vanilla_blocks::CHEST.default_state()
            );
            assert!(world.get_block_entity(CHEST_POS).is_some());
        }
        assert!(!Arc::ptr_eq(&a.game_time, &b.game_time));
        server.worlds.advance_domain_game_times();
        assert_eq!(a.game_time.ticks(), 1);
        assert_eq!(b.game_time.ticks(), 1);

        assert!(a.set_block(
            CHEST_POS,
            vanilla_blocks::AIR.default_state(),
            UpdateFlags::UPDATE_ALL
        ));
        assert_eq!(
            b.get_block_state(CHEST_POS),
            vanilla_blocks::CHEST.default_state(),
            "an edit in one clone does not reach another"
        );
        Ok(())
    })
}

#[test]
fn seed_override_needs_an_empty_generator_template() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("clone_seed_home");
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
        let flat_key = Identifier::new_static("clone_test", "flat");
        server.register_ready_template(chest_template(
            flat_key.clone(),
            Identifier::vanilla_static("flat"),
        ));
        let error = server
            .clone_world(
                &flat_key,
                Identifier::new_static("clone_seed", "arena"),
                DomainTarget::NewTemporary,
                CloneOverrides {
                    seed: Some(5),
                    ..CloneOverrides::default()
                },
            )
            .expect_err("a seed override must be refused for a flat template");
        assert!(error.contains("seed override"), "{error}");
        assert!(
            server
                .clone_world(
                    &Identifier::new_static("clone_test", "missing"),
                    Identifier::new_static("clone_seed", "arena"),
                    DomainTarget::NewTemporary,
                    CloneOverrides::default()
                )
                .is_err()
        );
        Ok(())
    })
}

#[test]
fn clone_joins_a_lasting_domain_and_refuses_missing_or_closing_domains() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("clone_lasting_home");
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
        let template_key = Identifier::new_static("clone_test", "template");
        server.register_ready_template(chest_template(
            template_key.clone(),
            Identifier::from_steel("empty"),
        ));
        let overrides = CloneOverrides {
            spawn: Some(BlockPos::new(0, 80, 0)),
            ..CloneOverrides::default()
        };

        let lasting_key = Identifier::new(home.domain().to_owned(), "clone".to_owned());
        let id = server.clone_world(
            &template_key,
            lasting_key.clone(),
            DomainTarget::Existing,
            overrides,
        )?;
        finish(&server, id).await?;
        let clone = server
            .worlds
            .get(&lasting_key)
            .ok_or("clone not published")?;
        assert!(clone.level_data.read().owns_game_time());
        assert!(!Arc::ptr_eq(&clone.game_time, &home.game_time));
        assert!(Arc::ptr_eq(
            clone
                .random_sequences
                .read()
                .as_ref()
                .ok_or("clone has no random sequences")?,
            home.random_sequences
                .read()
                .as_ref()
                .ok_or("home has no random sequences")?,
        ));
        assert_eq!(
            server
                .worlds
                .default_world(home.domain())
                .map(|w| w.key.clone()),
            Some(home.key.clone())
        );
        assert!(
            server.destroy_temporary_domain(home.domain()).is_err(),
            "a lasting domain is never destroyed"
        );

        assert!(
            server
                .clone_world(
                    &template_key,
                    Identifier::new_static("nowhere", "arena"),
                    DomainTarget::Existing,
                    overrides
                )
                .is_err(),
            "a domain that is not loaded is refused"
        );

        let temporary = Identifier::new_static("clone_closing", "arena");
        let id = server.clone_world(
            &template_key,
            temporary.clone(),
            DomainTarget::NewTemporary,
            overrides,
        )?;
        finish(&server, id).await?;
        let destroy = server.destroy_temporary_domain("clone_closing")?;
        let error = server
            .clone_world(
                &template_key,
                Identifier::new_static("clone_closing", "second"),
                DomainTarget::Existing,
                overrides,
            )
            .expect_err("a closing domain refuses new worlds");
        assert!(error.contains("closing"), "{error}");
        finish(&server, destroy).await?;
        Ok(())
    })
}
