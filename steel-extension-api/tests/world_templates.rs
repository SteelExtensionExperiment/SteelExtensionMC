//! World templates, clones and runtime world removal reaching a native extension.
//!
//! The probe in `tests/world-templates-probe` calls every binding through console lines; this
//! test builds a saved world on disk, drives the probe and checks the server's worlds.
#![cfg(feature = "host")]
#![recursion_limit = "256"]
mod common;

use common::{Fixture, Running, load_extensions, on_tick, start, stop, until};
use glam::DVec3;
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use steel_core::{
    chunk::test_support::insert_full_chunk,
    command::sender::CommandSender,
    entity::{SharedEntity, damage::DamageHistory, entities::PigEntity, next_entity_id},
    level_data::{GameTimeSource, WorldGenerationSettings},
    permission::PermissionSubjectIndex,
    server::WorldEntry,
    world::{LevelReader, World, WorldConfig, WorldStorageConfig},
    worldgen::{ChunkGeneratorType, EmptyChunkGenerator},
};
use steel_registry::{vanilla_blocks, vanilla_dimension_types, vanilla_entities};
use steel_utils::{
    BlockPos, ChunkPos, Identifier, WorldAabb,
    types::{Difficulty, GameType, UpdateFlags},
};
use tokio::{runtime::Runtime, time::sleep};
use uuid::Uuid;

const CHEST_POS: BlockPos = BlockPos::new(3, 65, 5);
const FLOOR_Y: i32 = 64;
const SPAWN_Y: &str = "65";
const WAIT: Duration = Duration::from_secs(25);
const SCAN_SIZE: f64 = 64.0;

/// Saves a one-chunk overworld with a stone floor, a chest and a pig under `directory`.
async fn build_saved_world(runtime: &Arc<Runtime>, directory: &Path) -> Result<(), String> {
    let dimension = &vanilla_dimension_types::OVERWORLD;
    let history = Arc::new(DamageHistory::default());
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .map_err(|e| e.to_string())?,
    );
    let settings = WorldGenerationSettings::from_generator_config(
        Identifier::from_steel("empty"),
        &toml::from_str("dimension_type='minecraft:overworld'").map_err(|e| e.to_string())?,
        dimension.key.clone(),
        dimension.min_y,
        dimension.height,
    );
    let world = World::new_with_config(
        Arc::clone(runtime),
        Identifier::new_static("source", "overworld"),
        dimension,
        4242,
        WorldConfig {
            damage_history: history,
            game_time_source: GameTimeSource::Primary,
            storage: WorldStorageConfig::Disk {
                path: directory.join("region").to_string_lossy().into_owned(),
            },
            level_data_path: Some(directory.to_string_lossy().into_owned()),
            generator: Arc::new(ChunkGeneratorType::Empty(EmptyChunkGenerator::new())),
            generation_settings: settings,
            view_distance: 2,
            simulation_distance: 2,
            max_chained_neighbor_updates: 1_000_000,
            compression: None,
            is_flat: false,
            sea_level: 0,
            default_gamemode: GameType::Survival,
            difficulty: Difficulty::Hard,
        },
        pool,
    )
    .await
    .map_err(|e| e.to_string())?;
    let _ = insert_full_chunk(&world, ChunkPos::new(0, 0), true);
    for x in 0..16 {
        for z in 0..16 {
            world.set_block(
                BlockPos::new(x, FLOOR_Y, z),
                vanilla_blocks::STONE.default_state(),
                UpdateFlags::UPDATE_ALL,
            );
        }
    }
    world.set_block(
        CHEST_POS,
        vanilla_blocks::CHEST.default_state(),
        UpdateFlags::UPDATE_ALL,
    );
    if world.get_block_entity(CHEST_POS).is_none() {
        return Err("the chest has no block entity".into());
    }
    let pig = Arc::new(PigEntity::new(
        &vanilla_entities::PIG,
        next_entity_id(),
        DVec3::new(4.5, 65.0, 4.5),
        Arc::downgrade(&world),
    ));
    world
        .try_add_entity(pig as SharedEntity)
        .map_err(|e| format!("{e:?}"))?;
    let mut saved = 0;
    world.cleanup(&mut saved).await;
    if saved == 0 {
        return Err("no chunk was saved".into());
    }
    world.chunk_map.stop_generation_refill_loop();
    world.chunk_map.task_tracker.close();
    world.chunk_map.task_tracker.wait().await;
    Ok(())
}

/// Hash of every file name and content below `directory`.
fn folder_hash(directory: &Path) -> Result<u64, String> {
    fn collect(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                collect(&path, files)?;
            } else {
                files.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    collect(directory, &mut files)?;
    files.sort();
    let mut hasher = DefaultHasher::new();
    for file in files {
        file.strip_prefix(directory)
            .map_err(|e| e.to_string())?
            .hash(&mut hasher);
        fs::read(&file)
            .map_err(|e| e.to_string())?
            .hash(&mut hasher);
    }
    Ok(hasher.finish())
}

struct Probe<'a> {
    running: &'a Running,
    log: PathBuf,
}

impl Probe<'_> {
    fn lines(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Submits `wt <line>` and waits for a log line starting with `expected`.
    async fn run(&self, line: &str, expected: &str) -> Result<String, String> {
        self.running
            .server
            .submit_command(CommandSender::Console, format!("wt {line}"), None)
            .map_err(|_| "command queue full".to_owned())?;
        self.wait_for(expected).await
    }

    async fn wait_for(&self, prefix: &str) -> Result<String, String> {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if let Some(line) = self.lines().into_iter().find(|l| l.starts_with(prefix)) {
                return Ok(line);
            }
            sleep(Duration::from_millis(10)).await;
        }
        Err(format!(
            "timed out waiting for {prefix}; log: {:?}",
            self.lines()
        ))
    }

    /// Polls the operation stored under `label` until it leaves `pending`; returns
    /// `(state, value)`.
    async fn finish(&self, label: &str) -> Result<(String, String), String> {
        let deadline = Instant::now() + WAIT;
        let prefix = format!("poll:{label}:");
        let mut seen = self.lines().len();
        while Instant::now() < deadline {
            self.running
                .server
                .submit_command(CommandSender::Console, format!("wt poll {label}"), None)
                .map_err(|_| "command queue full".to_owned())?;
            sleep(Duration::from_millis(60)).await;
            let lines = self.lines();
            for line in &lines[seen.min(lines.len())..] {
                if let Some(rest) = line.strip_prefix(&prefix) {
                    let (state, value) = rest.split_once(':').unwrap_or((rest, ""));
                    if state != "pending" {
                        return Ok((state.to_owned(), value.to_owned()));
                    }
                }
            }
            seen = lines.len();
        }
        Err(format!(
            "operation {label} did not finish: {:?}",
            self.lines()
        ))
    }

    /// Starts an operation through the probe and returns its terminal state and value.
    async fn operation(&self, line: &str, label: &str) -> Result<(String, String), String> {
        self.run(line, &format!("started:{label}")).await?;
        self.finish(label).await
    }
}

fn entities_in(world: &Arc<World>) -> usize {
    world
        .get_entities_in_aabb_matching(
            &WorldAabb::of_size(DVec3::new(8.0, 65.0, 8.0), SCAN_SIZE, SCAN_SIZE, SCAN_SIZE),
            |_| true,
        )
        .len()
}

fn assert_template_content(world: &Arc<World>) {
    assert_eq!(
        world.get_block_state(BlockPos::new(1, FLOOR_Y, 1)),
        vanilla_blocks::STONE.default_state()
    );
    assert_eq!(
        world.get_block_state(CHEST_POS),
        vanilla_blocks::CHEST.default_state()
    );
    assert!(
        world.get_block_entity(CHEST_POS).is_some(),
        "chest block entity"
    );
    assert_eq!(entities_in(world), 0, "entities are not cloned");
}

#[test]
#[ignore = "requires the world-templates-probe debug build (see the crate README)"]
#[expect(
    clippy::too_many_lines,
    reason = "one linear server session drives every world-template binding in order"
)]
fn templates_clone_create_and_remove_through_a_native_extension() -> Result<(), String> {
    let data_root =
        std::env::temp_dir().join(format!("steel-world-templates-{}", std::process::id()));
    let _ = fs::remove_dir_all(&data_root);
    let saved_world = data_root.join("saved-world");
    let empty_folder = data_root.join("empty-folder");
    fs::create_dir_all(&empty_folder).map_err(|e| e.to_string())?;

    let loaded = load_extensions(&[("world_templates_probe", "")], &data_root)?;
    let fixture = Fixture::new()?;
    let runtime = Arc::clone(&fixture.runtime);
    runtime.block_on(async {
        build_saved_world(&runtime, &saved_world).await?;
        let hash_before = folder_hash(&saved_world)?;

        let home = &fixture.world;
        let running = start(
            home,
            (loaded.extensions, loaded.commands),
            PermissionSubjectIndex::new(),
            Uuid::from_u128(0x7e57),
            "Tester",
        )
        .await?;
        let probe = Probe {
            running: &running,
            log: data_root.join("world-templates-probe/wt.log"),
        };
        let server = &running.server;

        // One good and one bad template, each with its own state.
        let (state, value) = probe
            .operation(
                &format!("load good wt:good {}", saved_world.display()),
                "good",
            )
            .await?;
        assert_eq!((state.as_str(), value.as_str()), ("ready", "wt:good"));
        let (state, message) = probe
            .operation(
                &format!("load bad wt:bad {}", empty_folder.display()),
                "bad",
            )
            .await?;
        assert_eq!(state, "failed");
        assert!(!message.is_empty());
        probe.run("status wt:good", "status:wt:good:ready:").await?;
        probe.run("status wt:bad", "status:wt:bad:failed:").await?;
        probe.run("status wt:never", "status:wt:never:none").await?;
        probe
            .run(
                &format!("load again wt:good {}", saved_world.display()),
                "refused:again",
            )
            .await?;

        // Clone into a new temporary domain and into the lasting domain.
        let (state, key) = probe
            .operation(
                &format!("clone c1 wt:good clone_a:arena new {SPAWN_Y}"),
                "c1",
            )
            .await?;
        assert_eq!((state.as_str(), key.as_str()), ("ready", "clone_a:arena"));
        let clone_a = server
            .worlds
            .get(&Identifier::new_static("clone_a", "arena"))
            .ok_or("clone a missing")?;
        assert_template_content(&clone_a);

        let (state, key) = probe
            .operation(
                &format!("clone c2 wt:good home:clone existing {SPAWN_Y}"),
                "c2",
            )
            .await?;
        assert_eq!((state.as_str(), key.as_str()), ("ready", "home:clone"));
        let lasting_clone = server
            .worlds
            .get(&Identifier::new_static("home", "clone"))
            .ok_or("lasting clone missing")?;
        assert_template_content(&lasting_clone);
        assert!(
            lasting_clone.game_time() < home.game_time(),
            "a clone starts its own clock at 0 after the domain default world"
        );

        // Two clones diverge when one is edited.
        let (state, _) = probe
            .operation(
                &format!("clone c3 wt:good clone_b:arena new {SPAWN_Y}"),
                "c3",
            )
            .await?;
        assert_eq!(state, "ready");
        let clone_b = server
            .worlds
            .get(&Identifier::new_static("clone_b", "arena"))
            .ok_or("clone b missing")?;
        let edited = Arc::clone(&clone_a);
        on_tick(server, move || {
            edited.set_block(
                CHEST_POS,
                vanilla_blocks::AIR.default_state(),
                UpdateFlags::UPDATE_ALL,
            );
            Ok(())
        })
        .await?;
        assert_eq!(
            clone_a.get_block_state(CHEST_POS),
            vanilla_blocks::AIR.default_state()
        );
        assert_template_content(&clone_b);

        // Generated worlds: a generator with defaults works, one needing configuration fails.
        let (state, _) = probe
            .operation(
                &format!("create g1 clone_c:flat minecraft:flat new {SPAWN_Y}"),
                "g1",
            )
            .await?;
        assert_eq!(state, "ready");
        probe
            .run(
                &format!("create g2 clone_d:arena steel:empty new {SPAWN_Y}"),
                "refused:g2:",
            )
            .await?;
        probe
            .run("removable home:clone", "removable:home:clone:true")
            .await?;
        probe
            .run("removable home:overworld", "removable:home:overworld:false")
            .await?;

        // A player in a clone is relocated when its world is removed.
        let player = Arc::clone(&running.player);
        let target = Arc::clone(&clone_b);
        let selecting = Arc::clone(server);
        on_tick(server, move || {
            selecting.queue_player_world_selection(player, &target, WorldEntry::Usual)
        })
        .await?;
        until(&running.player, "arrival in clone b", || {
            running.player.get_world().key == clone_b.key && running.player.has_joined_world()
        })
        .await?;
        let (state, key) = probe.operation("remove r1 clone_b:arena", "r1").await?;
        assert_eq!((state.as_str(), key.as_str()), ("ready", "clone_b:arena"));
        until(&running.player, "relocation out of clone b", || {
            running.player.get_world().key == home.key
        })
        .await?;
        assert!(
            server
                .worlds
                .get(&Identifier::new_static("clone_b", "arena"))
                .is_none()
        );

        // The lasting clone can go too; configured worlds cannot.
        let (state, _) = probe.operation("remove r2 home:clone", "r2").await?;
        assert_eq!(state, "ready");
        probe.run("remove r3 home:overworld", "refused:r3:").await?;

        stop(running, &loaded.initialized, home).await?;
        for world in [&clone_a, &lasting_clone] {
            world.chunk_map.stop_generation_refill_loop();
        }
        assert_eq!(
            folder_hash(&saved_world)?,
            hash_before,
            "the template folder was modified"
        );
        Ok(())
    })
}
