//! Host side of the text hologram API.
use super::events::parse_text;
use crate::{
    Position,
    runtime::{Billboard, HologramId},
};
use glam::DVec3;
use steel_core::{
    server::Server as CoreServer,
    world::{HologramError, HologramId as CoreHologramId},
};
use steel_utils::Identifier;

const UNKNOWN_HOLOGRAM: &str = "unknown or removed hologram";

pub(crate) fn create(
    server: &CoreServer,
    world: &str,
    position: Position,
    text: &str,
    billboard: Billboard,
) -> Result<HologramId, String> {
    let key: Identifier = world.parse().map_err(str::to_owned)?;
    let world = server
        .worlds
        .get(&key)
        .ok_or_else(|| format!("world {key} is not loaded"))?;
    world
        .create_hologram(
            DVec3::new(position.x, position.y, position.z),
            parse_text(text),
            billboard.into_host(),
        )
        .map(|id| HologramId { id: id.as_raw() })
        .map_err(|error: HologramError| error.to_string())
}

pub(crate) fn set_text(server: &CoreServer, id: HologramId, text: &str) -> Result<(), String> {
    let id = CoreHologramId::from_raw(id.id);
    // Ids are unique across worlds, so at most one world knows this one.
    server
        .worlds
        .values()
        .find_map(|world| world.set_hologram_text(id, parse_text(text)).ok())
        .ok_or_else(|| UNKNOWN_HOLOGRAM.to_owned())
}

pub(crate) fn remove(server: &CoreServer, id: HologramId) -> bool {
    let id = CoreHologramId::from_raw(id.id);
    server
        .worlds
        .values()
        .any(|world| world.remove_hologram(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::{common::Fixture, fresh_test_world};
    use std::{slice::from_ref, sync::Arc};
    use steel_core::{
        bootstrap::init_globals,
        config::{ResolvedDomainConfig, RuntimeConfig},
        entity::entities::{
            BillboardConstraints, Display as _, DisplayView as _, TextDisplayEntity,
        },
        permission::PermissionSubjectIndex,
        server::{Server, test_server_with_worlds_and_config},
    };
    use steel_utils::{Downcast as _, WorldAabb};

    const WORLD: &str = "home:overworld";
    const AT: Position = Position {
        x: 8.5,
        y: 70.0,
        z: 8.5,
    };

    fn server(fixture: &Fixture) -> Result<Arc<Server>, String> {
        let world = &fixture.world;
        let domain = ResolvedDomainConfig {
            name: "home".into(),
            default_world: world.key.clone(),
            worlds: vec![world.key.clone()],
        };
        fixture.runtime.block_on(test_server_with_worlds_and_config(
            "home".into(),
            from_ref(&domain),
            from_ref(world),
            PermissionSubjectIndex::new(),
            Arc::new(RuntimeConfig::default()),
            None,
        ))
    }

    /// Text and billboard of every text display in the origin chunk.
    fn displays(fixture: &Fixture) -> Vec<(String, BillboardConstraints)> {
        let chunk = WorldAabb::new(0.0, 0.0, 0.0, 16.0, 128.0, 16.0);
        fixture
            .world
            .get_entities_in_aabb(&chunk)
            .iter()
            .filter_map(|entity| {
                entity.downcast_ref::<TextDisplayEntity>().map(|display| {
                    let mut seen = None;
                    display.with_view(|view| {
                        seen = Some((view.text_ref().clone(), view.billboard_constraints()));
                    });
                    let (text, billboard) = seen.expect("with_view runs its closure");
                    (format!("{text:?}"), billboard)
                })
            })
            .collect()
    }

    #[test]
    fn create_failures_leave_no_entity() -> Result<(), String> {
        init_globals();
        let fixture = fresh_test_world("hologram_create_failures");
        let server = server(&fixture)?;

        let unloaded = create(&server, "home:missing", AT, "x", Billboard::Center);
        assert!(unloaded.is_err_and(|error| error.contains("home:missing")));
        let too_high = Position { y: 1.0e6, ..AT };
        assert!(create(&server, WORLD, too_high, "x", Billboard::Center).is_err());
        assert!(displays(&fixture).is_empty());
        Ok(())
    }

    #[test]
    fn handle_lifecycle_updates_one_entity_and_dies_on_remove() -> Result<(), String> {
        init_globals();
        let fixture = fresh_test_world("hologram_lifecycle");
        let server = server(&fixture)?;

        let id = create(&server, WORLD, AT, "before", Billboard::Vertical)?;
        let before = displays(&fixture);
        assert_eq!(before.len(), 1);
        assert!(before[0].0.contains("before"));
        assert!(matches!(before[0].1, BillboardConstraints::Vertical));

        set_text(&server, id, r#"{"text":"after"}"#)?;
        let after = displays(&fixture);
        assert_eq!(after.len(), 1);
        assert!(after[0].0.contains("after"));

        assert!(remove(&server, id));
        assert!(!remove(&server, id));
        assert!(displays(&fixture).is_empty());
        assert_eq!(set_text(&server, id, "late"), Err(UNKNOWN_HOLOGRAM.into()));
        Ok(())
    }
}
