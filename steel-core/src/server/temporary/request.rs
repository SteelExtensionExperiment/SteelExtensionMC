//! Requests for runtime worlds: what a world is built from and where it joins.
use super::TemporaryWorldRequest;
use crate::world::template::WorldTemplate;
use std::sync::Arc;
use steel_utils::{
    BlockPos, Identifier,
    types::{Difficulty, GameType},
};

/// What a runtime world's chunks come from.
pub enum WorldSource {
    /// A registered generator with its configuration.
    Generator {
        /// Registered world generator.
        generator: Identifier,
        /// Generator-specific configuration.
        config: toml::Value,
    },
    /// A saved world; chunks the clone has not saved are read from the template.
    Template(Arc<WorldTemplate>),
}

impl WorldSource {
    /// The generator and configuration used to validate the source and to generate unsaved chunks.
    pub(super) fn generator_config(&self) -> (&Identifier, &toml::Value) {
        match self {
            Self::Generator { generator, config } => (generator, config),
            Self::Template(template) => {
                (&template.generation.generator, &template.generation.config)
            }
        }
    }
}

/// Where a runtime world joins the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainTarget {
    /// First world of a new temporary domain. It becomes the domain's default world, owns its
    /// clock and creates the domain's loot random sequences.
    NewTemporary,
    /// Additional world of a temporary domain with a clock derived from the domain default
    /// world. Only `create_temporary_world` uses this.
    TemporarySibling,
    /// Additional world of a loaded domain, lasting or temporary and not closing. It owns its
    /// clock and shares the domain's loot random sequences, so the domain needs a default world.
    Existing,
}

/// A RAM-only world to prepare and publish at a tick boundary.
pub struct RuntimeWorldRequest {
    /// Unique `domain:world` identity.
    pub key: Identifier,
    /// Where the world's chunks come from.
    pub source: WorldSource,
    /// Where the world joins.
    pub target: DomainTarget,
    /// World seed.
    pub seed: i64,
    /// Spawn position.
    pub spawn: BlockPos,
    /// Spawn angle (yaw).
    pub spawn_angle: f32,
    /// Game mode for first visits to this world.
    pub default_gamemode: GameType,
    /// Initial world difficulty.
    pub difficulty: Difficulty,
}

impl From<TemporaryWorldRequest> for RuntimeWorldRequest {
    fn from(request: TemporaryWorldRequest) -> Self {
        Self {
            key: request.key,
            source: WorldSource::Generator {
                generator: request.generator,
                config: request.config,
            },
            target: if request.primary {
                DomainTarget::NewTemporary
            } else {
                DomainTarget::TemporarySibling
            },
            seed: request.seed,
            spawn: request.spawn,
            spawn_angle: 0.0,
            default_gamemode: request.default_gamemode,
            difficulty: request.difficulty,
        }
    }
}
