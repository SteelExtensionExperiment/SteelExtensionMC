//! Cloning a loaded template into a runtime world.
use super::{DomainTarget, RuntimeWorldRequest, WorldSource};
use crate::server::{Server, templates::TemplateEntry, world_operations::WorldOperationId};
use std::sync::Arc;
use steel_utils::{
    BlockPos, Identifier,
    types::{Difficulty, GameType},
};

/// Values that replace the template's defaults for one clone.
#[derive(Debug, Default, Clone, Copy)]
pub struct CloneOverrides {
    /// World seed. Only allowed for `steel:empty` templates.
    pub seed: Option<i64>,
    /// Spawn position.
    pub spawn: Option<BlockPos>,
    /// Game mode for first visits. Not stored in a template; defaults to the server default
    /// world's.
    pub default_gamemode: Option<GameType>,
    /// World difficulty.
    pub difficulty: Option<Difficulty>,
}

impl Server {
    /// Starts a clone of the loaded template `template` named `key`. The clone owns its clock,
    /// starts at game time 0 and has no entities.
    ///
    /// # Errors
    /// Fails immediately, without an operation, when the template is not `Ready`, `target` would
    /// give the clone a derived clock, a seed override is set for a template whose generator is
    /// not `steel:empty`, or the request is refused as in [`Server::create_runtime_world`].
    pub fn clone_world(
        self: &Arc<Self>,
        template: &Identifier,
        key: Identifier,
        target: DomainTarget,
        overrides: CloneOverrides,
    ) -> Result<WorldOperationId, String> {
        if target == DomainTarget::TemporarySibling {
            return Err("a clone owns its clock and cannot derive one".into());
        }
        let template = match self.world_template(template) {
            Some(TemplateEntry::Ready(template)) => template,
            Some(TemplateEntry::Loading) => {
                return Err(format!("world template {template} is still loading"));
            }
            Some(TemplateEntry::Failed(message)) => {
                return Err(format!(
                    "world template {template} failed to load: {message}"
                ));
            }
            None => return Err(format!("unknown world template {template}")),
        };
        if overrides.seed.is_some()
            && template.generation.generator != Identifier::from_steel("empty")
        {
            return Err(format!(
                "a seed override needs a steel:empty template; {template_key} generates unsaved chunks with {generator}",
                template_key = template.key,
                generator = template.generation.generator,
            ));
        }
        let (spawn, spawn_angle) = overrides.spawn.map_or_else(
            || {
                (
                    BlockPos::new(template.spawn.x, template.spawn.y, template.spawn.z),
                    template.spawn.angle,
                )
            },
            |spawn| (spawn, 0.0),
        );
        self.create_runtime_world(RuntimeWorldRequest {
            key,
            seed: overrides.seed.unwrap_or(template.seed),
            spawn,
            spawn_angle,
            default_gamemode: overrides
                .default_gamemode
                .unwrap_or_else(|| self.overworld().default_gamemode()),
            difficulty: overrides.difficulty.unwrap_or(template.difficulty),
            source: WorldSource::Template(template),
            target,
        })
    }
}
