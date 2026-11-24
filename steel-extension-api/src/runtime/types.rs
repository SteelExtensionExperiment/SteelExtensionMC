//! Commands and runtime operations on temporary, domain-isolated worlds and players.
use crate::BlockPos;
use stabby::{option::Option as StableOption, string::String as ApiString};
#[cfg(feature = "host")]
use steel_core::entity::entities::BillboardConstraints;
use steel_core::server::temporary::{CloneOverrides, DomainTarget};
#[cfg(feature = "host")]
use steel_utils::{
    BlockPos as CoreBlockPos,
    types::{Difficulty as CoreDifficulty, GameType as CoreGameMode},
};
/// Host-resolved player identity. A lookup always checks current session ownership.
#[stabby::stabby]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerId {
    pub(crate) bytes: [u8; 16],
    pub(crate) entity_id: i32,
}
/// World coordinates in blocks.
#[stabby::stabby]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Position {
    /// X coordinate.
    pub x: f64,
    /// Y coordinate.
    pub y: f64,
    /// Z coordinate.
    pub z: f64,
}
/// Snapshot of a resident player, including server-controlled actors.
#[stabby::stabby]
pub struct PlayerInfo {
    /// Runtime player identity.
    pub id: PlayerId,
    /// Display name.
    pub name: ApiString,
    /// Current world key.
    pub world: ApiString,
    /// Position at snapshot time.
    pub position: Position,
    /// Whether the host supplies actions.
    pub bot: bool,
    /// Current health.
    pub health: f32,
}
/// World operation identity; completed results are consumed by `operation_status`.
#[stabby::stabby]
#[derive(Debug, Clone, Copy)]
pub struct Operation {
    pub(crate) id: u64,
}
/// Opaque handle of a text hologram, valid until it is removed or its world unloads.
#[stabby::stabby]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HologramId {
    pub(crate) id: u64,
}
/// How a hologram text turns towards the viewer, matching the vanilla billboard constraints.
#[stabby::stabby]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Billboard {
    /// Both axes fixed; the text keeps its orientation.
    Fixed,
    /// Only the vertical axis is fixed; the text turns around it.
    Vertical,
    /// Only the horizontal axis is fixed; the text tilts up and down.
    Horizontal,
    /// Neither axis is fixed; the text always faces the viewer.
    Center,
}
/// State of a world operation.
#[stabby::stabby]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationState {
    /// Preparation or cleanup is still running.
    Pending,
    /// Completed successfully.
    Ready,
    /// Failed with a diagnostic.
    Failed,
}
/// A completed status consumes the operation result.
#[stabby::stabby]
pub struct OperationStatus {
    /// World/domain key on success, diagnostic on failure; empty while pending.
    pub value: ApiString,
    /// Current operation state.
    pub state: OperationState,
}
/// Player game mode.
#[stabby::stabby]
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum GameMode {
    /// Normal survival rules.
    Survival,
    /// Creative controls and inventory rules.
    Creative,
    /// Adventure permissions.
    Adventure,
    /// Flying observer without collisions.
    Spectator,
}
/// Initial world difficulty.
#[stabby::stabby]
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum Difficulty {
    /// Peaceful rules.
    Peaceful,
    /// Easy rules.
    Easy,
    /// Normal rules.
    Normal,
    /// Hard rules.
    Hard,
}
/// Explicit gameplay and spawn settings for a temporary world.
#[stabby::stabby]
#[derive(Debug, Clone, Copy)]
pub struct WorldSettings {
    /// Seed passed to the registered generator.
    pub seed: i64,
    /// Spawn chosen by the map author.
    pub spawn: BlockPos,
    /// Game mode for players visiting this world for the first time.
    pub default_game_mode: GameMode,
    /// Initial world difficulty.
    pub difficulty: Difficulty,
}
/// Load state of a registered world template.
#[stabby::stabby]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateState {
    /// The saved world is still being read.
    Loading,
    /// The template can be cloned.
    Ready,
    /// Loading failed; the key stays taken.
    Failed,
}
/// Status of a world template.
#[stabby::stabby]
pub struct TemplateStatus {
    /// Current load state.
    pub state: TemplateState,
    /// Diagnostic when `Failed`, empty otherwise.
    pub message: ApiString,
    /// Number of saved chunks held by the template; zero until `Ready`.
    pub chunks: u64,
}
/// Where a cloned or generated world joins the server.
#[stabby::stabby]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneTarget {
    /// A new temporary domain named by the key's namespace; the world becomes its default world.
    NewTemporaryDomain,
    /// A loaded domain named by the key's namespace, lasting or temporary and not closing.
    ExistingDomain,
}
/// Values that replace a template's defaults for one clone.
#[stabby::stabby]
#[derive(Debug, Clone)]
pub struct CloneSettings {
    /// Seed. Only accepted for templates generated by `steel:empty`.
    pub seed: StableOption<i64>,
    /// Spawn position.
    pub spawn: StableOption<BlockPos>,
    /// Game mode for first visits; the server default for new worlds when absent.
    pub default_game_mode: StableOption<GameMode>,
    /// World difficulty.
    pub difficulty: StableOption<Difficulty>,
}
#[cfg(feature = "host")]
impl CloneTarget {
    pub(crate) const fn into_host(self) -> DomainTarget {
        match self {
            Self::NewTemporaryDomain => DomainTarget::NewTemporary,
            Self::ExistingDomain => DomainTarget::Existing,
        }
    }
}
#[cfg(feature = "host")]
impl CloneSettings {
    pub(crate) fn into_host(self) -> CloneOverrides {
        let spawn: Option<BlockPos> = self.spawn.into();
        let default_game_mode: Option<GameMode> = self.default_game_mode.into();
        let difficulty: Option<Difficulty> = self.difficulty.into();
        let seed: Option<i64> = self.seed.into();
        CloneOverrides {
            seed,
            spawn: spawn.map(|pos| CoreBlockPos::new(pos.x, pos.y, pos.z)),
            default_gamemode: default_game_mode.map(GameMode::into_host),
            difficulty: difficulty.map(Difficulty::into_host),
        }
    }
}
#[cfg(feature = "host")]
impl GameMode {
    pub(crate) const fn into_host(self) -> CoreGameMode {
        match self {
            Self::Survival => CoreGameMode::Survival,
            Self::Creative => CoreGameMode::Creative,
            Self::Adventure => CoreGameMode::Adventure,
            Self::Spectator => CoreGameMode::Spectator,
        }
    }
}
#[cfg(feature = "host")]
impl Billboard {
    pub(crate) const fn into_host(self) -> BillboardConstraints {
        match self {
            Self::Fixed => BillboardConstraints::Fixed,
            Self::Vertical => BillboardConstraints::Vertical,
            Self::Horizontal => BillboardConstraints::Horizontal,
            Self::Center => BillboardConstraints::Center,
        }
    }
}
#[cfg(feature = "host")]
impl Difficulty {
    pub(crate) const fn into_host(self) -> CoreDifficulty {
        match self {
            Self::Peaceful => CoreDifficulty::Peaceful,
            Self::Easy => CoreDifficulty::Easy,
            Self::Normal => CoreDifficulty::Normal,
            Self::Hard => CoreDifficulty::Hard,
        }
    }
}
/// Controller intent for one bot tick.
#[stabby::stabby]
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerControls {
    /// Move forward.
    pub forward: bool,
    /// Move backward.
    pub backward: bool,
    /// Strafe left.
    pub left: bool,
    /// Strafe right.
    pub right: bool,
    /// Jump.
    pub jump: bool,
    /// Sneak.
    pub sneak: bool,
    /// Sprint.
    pub sprint: bool,
}
/// Bot controls expire after one actor tick unless supplied again.
#[stabby::stabby]
pub struct BotInput {
    /// Horizontal rotation in degrees.
    pub yaw: f32,
    /// Vertical rotation in degrees.
    pub pitch: f32,
    /// Movement intent.
    pub movement: PlayerControls,
    /// Block to mine with normal tool speed and reach.
    pub dig: StableOption<BlockPos>,
}
#[cfg(all(test, feature = "host"))]
mod tests {
    use super::*;

    #[test]
    fn clone_settings_convert_each_override_and_leave_absent_ones_unset() {
        let overrides = CloneSettings {
            seed: StableOption::Some(7),
            spawn: StableOption::Some(BlockPos { x: 1, y: 70, z: -3 }),
            default_game_mode: StableOption::None(),
            difficulty: StableOption::Some(Difficulty::Hard),
        }
        .into_host();

        assert_eq!(overrides.seed, Some(7));
        assert_eq!(overrides.spawn, Some(CoreBlockPos::new(1, 70, -3)));
        assert_eq!(overrides.default_gamemode, None);
        assert_eq!(overrides.difficulty, Some(CoreDifficulty::Hard));
    }

    #[test]
    fn clone_targets_map_to_host_domain_targets() {
        assert_eq!(
            CloneTarget::NewTemporaryDomain.into_host(),
            DomainTarget::NewTemporary
        );
        assert_eq!(
            CloneTarget::ExistingDomain.into_host(),
            DomainTarget::Existing
        );
    }
}
