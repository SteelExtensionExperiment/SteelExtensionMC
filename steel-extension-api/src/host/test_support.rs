//! Native RAM-only world fixtures shared by SDK policy tests.

#[path = "../../tests/common/mod.rs"]
pub(super) mod common;

use std::sync::{Arc, Weak};
use steel_core::{
    chunk::test_support::insert_full_chunk,
    config::RuntimeConfig,
    player::{ClientInformation, Player, PlayerConnection, PlayerSession},
    world::World,
};
use steel_registry::game_profile::GameProfile;
use steel_utils::ChunkPos;

pub(super) fn fresh_test_world(_key: &'static str) -> common::Fixture {
    common::Fixture::new().expect("native test world should initialize")
}

pub(super) fn insert_ready_full_chunk(world: &Arc<World>, pos: ChunkPos) {
    if world.chunk_map.with_full_chunk(pos, |_| ()).is_none() {
        let _ = insert_full_chunk(world, pos, true);
    }
}

pub(super) struct TestPlayerBuilder {
    world: Arc<World>,
    name: String,
    id: i32,
}
impl TestPlayerBuilder {
    pub(super) fn new(world: Arc<World>, name: impl Into<String>, id: i32) -> Self {
        Self {
            world,
            name: name.into(),
            id,
        }
    }
    pub(super) fn build(self) -> Arc<Player> {
        let config = Arc::new(RuntimeConfig {
            view_distance: 2,
            simulation_distance: 2,
            packet_workers: Some(1),
            chunk_generation_threads: Some(1),
            chunk_encoding_threads: Some(1),
            ..RuntimeConfig::default()
        });
        let session = Arc::new(PlayerSession::new(
            config.chat_spam_threshold_seconds,
            config.command_spam_threshold_seconds,
        ));
        let profile = GameProfile::new(uuid::Uuid::new_v4(), self.name, Vec::new())
            .expect("test player name should be valid");
        let player = Arc::new(Player::new(
            profile,
            Arc::new(PlayerConnection::Other(Box::new(
                common::PeerConnection::default(),
            ))),
            Arc::clone(&session),
            self.world,
            Weak::new(),
            config,
            self.id,
            ClientInformation::default(),
        ));
        assert!(session.bind_initial_player(&player));
        player
    }
}
