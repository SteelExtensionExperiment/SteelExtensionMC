//! Drives every world-template binding from a native extension through console lines
//! `wt <verb> <arguments>` and records each outcome in `<data>/wt.log`. Not meant for servers.
//!
//! | Line | Binding |
//! | --- | --- |
//! | `wt load <label> <key> <path>` | `load_world_template` |
//! | `wt status <key>` | `world_template_status` |
//! | `wt clone <label> <template> <key> <new\|existing> <spawn_y>` | `clone_world` |
//! | `wt create <label> <key> <generator> <new\|existing> <spawn_y>` | `create_world` |
//! | `wt remove <label> <key>` | `remove_world` |
//! | `wt removable <key>` | `world_is_removable` |
//! | `wt poll <label>` | `operation_status` of the operation stored under `label` |
#![recursion_limit = "256"]

use std::{
    collections::HashMap,
    io::Write as _,
    sync::{Mutex, OnceLock, PoisonError},
};
use steel_extension_api::{
    ApiResult, BlockPos, CloneSettings, CloneTarget, Difficulty, ExtensionContext, GameMode, Hooks,
    Operation, OperationState, Server, TemplateState, TemplateStatus, WorldSettings, scheduler,
    stabby::option::Option as StableOption,
};

static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
static OPERATIONS: OnceLock<Mutex<HashMap<String, Operation>>> = OnceLock::new();

fn record(line: &str) {
    if let Some(file) = LOG.get() {
        let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = writeln!(file, "{line}");
    }
}

fn operations() -> std::sync::MutexGuard<'static, HashMap<String, Operation>> {
    OPERATIONS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Logs the start result and keeps the operation under `label` for `wt poll`.
fn started(label: &str, result: ApiResult<Operation>) {
    match Result::from(result) {
        Ok(operation) => {
            operations().insert(label.to_owned(), operation);
            record(&format!("started:{label}"));
        }
        Err(error) => record(&format!("refused:{label}:{error}")),
    }
}

fn target(word: &str) -> Result<CloneTarget, String> {
    match word {
        "new" => Ok(CloneTarget::NewTemporaryDomain),
        "existing" => Ok(CloneTarget::ExistingDomain),
        other => Err(format!("unknown target {other}")),
    }
}

fn spawn(y: &str) -> Result<BlockPos, String> {
    Ok(BlockPos {
        x: 0,
        y: y.parse().map_err(|_| format!("bad spawn y {y}"))?,
        z: 0,
    })
}

fn run(server: &Server<'_>, words: &[&str]) -> Result<(), String> {
    match words {
        ["load", label, key, path] => {
            started(label, server.load_world_template((*key).into(), (*path).into()));
        }
        ["status", key] => {
            let status: Option<TemplateStatus> = server.world_template_status((*key).into()).into();
            match status {
                None => record(&format!("status:{key}:none")),
                Some(status) => record(&format!(
                    "status:{key}:{}:{}:{}",
                    match status.state {
                        TemplateState::Loading => "loading",
                        TemplateState::Ready => "ready",
                        TemplateState::Failed => "failed",
                    },
                    status.chunks,
                    status.message
                )),
            }
        }
        ["clone", label, template, key, which, spawn_y] => {
            let settings = CloneSettings {
                seed: StableOption::None(),
                spawn: StableOption::Some(spawn(spawn_y)?),
                default_game_mode: StableOption::None(),
                difficulty: StableOption::None(),
            };
            started(
                label,
                server.clone_world(
                    (*template).into(),
                    (*key).into(),
                    target(which)?,
                    settings,
                ),
            );
        }
        ["create", label, key, generator, which, spawn_y] => {
            let settings = WorldSettings {
                seed: 0,
                spawn: spawn(spawn_y)?,
                default_game_mode: GameMode::Adventure,
                difficulty: Difficulty::Peaceful,
            };
            started(
                label,
                server.create_world(
                    (*key).into(),
                    (*generator).into(),
                    target(which)?,
                    settings,
                ),
            );
        }
        ["remove", label, key] => started(label, server.remove_world((*key).into())),
        ["removable", key] => {
            record(&format!("removable:{key}:{}", server.world_is_removable((*key).into())));
        }
        ["poll", label] => {
            let operation = *operations()
                .get(*label)
                .ok_or_else(|| format!("unknown operation {label}"))?;
            let status = Result::from(server.operation_status(operation))
                .map_err(|error| error.to_string())?;
            let state = match status.state {
                OperationState::Pending => "pending",
                OperationState::Ready => "ready",
                OperationState::Failed => "failed",
            };
            record(&format!("poll:{label}:{state}:{}", status.value));
        }
        other => return Err(format!("unknown line {other:?}")),
    }
    Ok(())
}

fn pre_init(mut hooks: Hooks<'_>) -> Result<(), String> {
    hooks.enable_scheduler();
    hooks.on_server_command(
        steel_extension_api::events::EventPriority::Normal,
        |event| {
            let line = event.command().to_string();
            let Some(rest) = line.strip_prefix("wt ") else {
                return;
            };
            event.set_cancelled(true);
            let rest = rest.to_owned();
            scheduler::run_later(1, move |server| {
                let words: Vec<&str> = rest.split_whitespace().collect();
                if let Err(error) = run(server, &words) {
                    record(&format!("error:{error}"));
                }
            });
        },
    );
    Ok(())
}

fn init(context: ExtensionContext<'_>) -> Result<(), String> {
    let path = std::path::Path::new(context.data_directory().as_str()).join("wt.log");
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    LOG.set(Mutex::new(file))
        .map_err(|_| "probe initialized twice".to_owned())
}

steel_extension_api::export_extension!("world-templates-probe", pre_init, init);
