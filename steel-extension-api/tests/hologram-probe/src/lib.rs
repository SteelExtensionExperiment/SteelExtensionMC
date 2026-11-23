//! Drives the hologram bindings through `probe:holo <action>` and records each outcome in
//! `<data>/holograms.log`, so integration tests can check the results. Not meant for servers.
use std::{
    io::Write as _,
    sync::{Mutex, OnceLock, PoisonError},
};
use steel_extension_api::{
    ApiResult, Billboard, BlockPos, Command, Commands, Difficulty, ExtensionContext, GameMode,
    GeneratorRegistry, HologramId, Hooks, Operation, OperationState, Position, Server, Terrain,
    WorldSettings,
    stabby::{self, str::Str, string::String as ApiString},
};

const WORLD: &str = "home:overworld";
const AT: Position = Position {
    x: 8.5,
    y: 66.0,
    z: 8.5,
};
const CREATED_TEXT: &str = "stats";
const UPDATED_TEXT: &str = "updated";

const TEMP_DOMAIN: &str = "holotemp";
const TEMP_WORLD: &str = "holotemp:arena";
const TEMP_GENERATOR: &str = "probe:void";

static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
/// The last created handles, `[home world, temporary world]`; kept after `remove` or the
/// world's end so the test can reuse the dead handle.
static HOLOGRAMS: Mutex<[Option<HologramId>; 2]> = Mutex::new([None; 2]);
static OPERATION: Mutex<Option<Operation>> = Mutex::new(None);

fn record(line: &str) {
    if let Some(file) = LOG.get() {
        let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = writeln!(file, "{line}");
    }
}

fn handle(slot: usize) -> Result<HologramId, String> {
    let held = HOLOGRAMS.lock().unwrap_or_else(PoisonError::into_inner);
    held[slot].ok_or_else(|| "no hologram was created".to_owned())
}

fn create(server: &Server<'_>, slot: usize, world: &str) -> Result<String, String> {
    let id = Result::from(server.create_hologram(
        world.into(),
        AT,
        CREATED_TEXT.into(),
        Billboard::Center,
    ))
    .map_err(|error| error.to_string())?;
    HOLOGRAMS.lock().unwrap_or_else(PoisonError::into_inner)[slot] = Some(id);
    Ok("ok".into())
}

fn set(server: &Server<'_>, slot: usize) -> Result<String, String> {
    Result::from(server.set_hologram_text(handle(slot)?, UPDATED_TEXT.into()))
        .map(|()| "ok".to_owned())
        .map_err(|error| error.to_string())
}

fn start_operation(operation: ApiResult<Operation>) -> Result<String, String> {
    let operation = Result::from(operation).map_err(|error| error.to_string())?;
    *OPERATION.lock().unwrap_or_else(PoisonError::into_inner) = Some(operation);
    Ok("started".into())
}

fn operation_state(server: &Server<'_>) -> Result<String, String> {
    let operation = (*OPERATION.lock().unwrap_or_else(PoisonError::into_inner))
        .ok_or("no operation was started")?;
    let status = Result::from(server.operation_status(operation)).map_err(|e| e.to_string())?;
    Ok(match status.state {
        OperationState::Pending => "pending".to_owned(),
        OperationState::Ready => "ready".to_owned(),
        OperationState::Failed => format!("failed {}", status.value),
    })
}

fn run(context: &Command<'_>, action: &str) -> Result<String, String> {
    let server = context.server();
    match action {
        "create" => create(&server, 0, WORLD),
        "set" => set(&server, 0),
        "remove" => Result::from(server.remove_hologram(handle(0)?))
            .map(|existed| existed.to_string())
            .map_err(|error| error.to_string()),
        "temp_world" => start_operation(server.create_temporary_world(
            TEMP_WORLD.into(),
            TEMP_GENERATOR.into(),
            WorldSettings {
                seed: 0,
                spawn: BlockPos { x: 0, y: 64, z: 0 },
                default_game_mode: GameMode::Survival,
                difficulty: Difficulty::Peaceful,
            },
            true,
        )),
        "temp_create" => create(&server, 1, TEMP_WORLD),
        "temp_set" => set(&server, 1),
        "temp_destroy" => start_operation(server.destroy_temporary_domain(TEMP_DOMAIN.into())),
        "operation" => operation_state(&server),
        _ => Err(format!("unknown action {action}")),
    }
}

#[stabby::export(callback)]
fn command(context: Command<'_>, action: Str<'_>) -> ApiResult<i32> {
    let action = action.as_str();
    match run(&context, action) {
        Ok(outcome) => {
            record(&format!("{action}:{outcome}"));
            Ok::<_, ApiString>(1).into()
        }
        Err(error) => {
            record(&format!("{action}:err:{error}"));
            Err::<i32, _>(ApiString::from(error)).into()
        }
    }
}

#[stabby::export(callback)]
fn commands(mut registry: Commands<'_>) -> ApiResult<()> {
    registry.register_player_command(
        "probe:holo".into(),
        [
            Str::new("create"),
            Str::new("set"),
            Str::new("remove"),
            Str::new("temp_world"),
            Str::new("temp_create"),
            Str::new("temp_set"),
            Str::new("temp_destroy"),
            Str::new("operation"),
        ]
        .as_slice()
        .into(),
        command,
    )
}

#[stabby::export(callback)]
fn void(_: Terrain<'_>) {}

#[stabby::export(callback)]
fn generators(mut registry: GeneratorRegistry<'_>) -> ApiResult<()> {
    registry.register_terrain_generator(
        TEMP_GENERATOR.into(),
        "minecraft:overworld".into(),
        "minecraft:plains".into(),
        0,
        void,
    )
}

fn pre_init(mut hooks: Hooks<'_>) -> Result<(), String> {
    Result::from(hooks.register_commands(commands)).map_err(|error| error.to_string())?;
    Result::from(hooks.register_generator_registration(generators))
        .map_err(|error| error.to_string())
}

fn init(context: ExtensionContext<'_>) -> Result<(), String> {
    let path = std::path::Path::new(context.data_directory().as_str()).join("holograms.log");
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    LOG.set(Mutex::new(file))
        .map_err(|_| "probe initialized twice".to_owned())
}

steel_extension_api::export_extension!("hologram-probe", pre_init, init);
