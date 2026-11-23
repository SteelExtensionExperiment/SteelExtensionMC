//! Registry of read-only world templates and their asynchronous loading.
use std::{path::PathBuf, sync::Arc};

use rustc_hash::FxHashMap;
use steel_utils::{Identifier, locks::SyncMutex};

use super::{Server, world_operations::WorldOperationId};
use crate::world::template::WorldTemplate;

/// State of a registered template. `Ready` and `Failed` last for the process lifetime; a retry
/// needs a new key.
#[derive(Clone)]
pub enum TemplateEntry {
    /// The load task is still running.
    Loading,
    /// The template is loaded and shared by every clone.
    Ready(Arc<WorldTemplate>),
    /// The load failed with this diagnostic.
    Failed(String),
}

#[derive(Default)]
pub(super) struct WorldTemplates {
    entries: SyncMutex<FxHashMap<Identifier, TemplateEntry>>,
}

impl Server {
    /// Starts loading the saved world in `path` as template `key`. The returned operation
    /// finishes with the key once the template is `Ready`, or with the diagnostic on failure.
    ///
    /// # Errors
    /// Fails immediately when `key` already has an entry in any state or `path` is not a directory.
    pub fn load_world_template(
        self: &Arc<Self>,
        key: Identifier,
        path: PathBuf,
    ) -> Result<WorldOperationId, String> {
        if !path.is_dir() {
            return Err(format!("{} is not a directory", path.display()));
        }
        let id = {
            let mut entries = self.world_templates.entries.lock();
            if entries.contains_key(&key) {
                return Err(format!("world template {key} already exists"));
            }
            let id = self.world_operations.allocate()?;
            entries.insert(key.clone(), TemplateEntry::Loading);
            id
        };
        let server = Arc::clone(self);
        self.temporary_worlds.cleanup.spawn(async move {
            let pool = Arc::clone(&server.overworld().chunk_map.generation_pool);
            let result = WorldTemplate::load(key.clone(), &path, &server.world_generators, &pool)
                .await
                .map(Arc::new);
            let operation_result = match &result {
                Ok(_) => Ok(key.clone()),
                Err(message) => Err(message.clone()),
            };
            let entry = match result {
                Ok(template) => TemplateEntry::Ready(template),
                Err(message) => TemplateEntry::Failed(message),
            };
            server.world_templates.entries.lock().insert(key, entry);
            server.world_operations.finish(id, operation_result);
        });
        Ok(id)
    }

    /// The registry entry for `key`, if any.
    #[must_use]
    pub fn world_template(&self, key: &Identifier) -> Option<TemplateEntry> {
        self.world_templates.entries.lock().get(key).cloned()
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, slice::from_ref, time::Duration};

    use steel_registry::vanilla_dimension_types::OVERWORLD;
    use tokio::{runtime::Builder, time::sleep};

    use super::*;
    use crate::{
        bootstrap::init_globals,
        config::ResolvedDomainConfig,
        permission::PermissionSubjectIndex,
        server::test_server_with_worlds_and_config,
        test_support::{fresh_test_world, test_runtime_config},
        world::template::tests::{flat_settings, temp_world_dir, write_level},
    };

    async fn wait_for(server: &Arc<Server>, id: WorldOperationId) -> Result<Identifier, String> {
        for _ in 0..10_000 {
            if let Some(result) = server.take_world_operation(id)? {
                return result;
            }
            sleep(Duration::from_millis(1)).await;
        }
        Err("template load did not finish".into())
    }

    #[test]
    fn duplicates_are_rejected_and_each_template_has_its_own_state() -> Result<(), String> {
        init_globals();
        let fixture = fresh_test_world("template_registry_home");
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
            let good_dir = temp_world_dir("registry-good");
            write_level(
                &good_dir,
                Some(flat_settings(OVERWORLD.min_y, OVERWORLD.height)),
            );
            let bad_dir = temp_world_dir("registry-bad");
            let good = Identifier::new_static("test", "good");
            let bad = Identifier::new_static("test", "bad");

            let good_id = server.load_world_template(good.clone(), good_dir.clone())?;
            assert!(
                matches!(server.world_template(&good), Some(TemplateEntry::Loading)),
                "loading must not block the caller"
            );
            let bad_id = server.load_world_template(bad.clone(), bad_dir.clone())?;
            assert!(
                server
                    .load_world_template(good.clone(), good_dir.clone())
                    .is_err()
            );
            assert!(
                server
                    .load_world_template(
                        Identifier::new_static("test", "file"),
                        good_dir.join("level.toml")
                    )
                    .is_err(),
                "a path that is not a directory fails immediately"
            );

            assert_eq!(wait_for(&server, good_id).await?, good);
            assert!(wait_for(&server, bad_id).await.is_err());
            assert!(matches!(
                server.world_template(&good),
                Some(TemplateEntry::Ready(_))
            ));
            assert!(matches!(
                server.world_template(&bad),
                Some(TemplateEntry::Failed(_))
            ));
            assert!(
                server.load_world_template(bad, bad_dir.clone()).is_err(),
                "a failed key stays taken"
            );
            let _ = fs::remove_dir_all(&good_dir);
            let _ = fs::remove_dir_all(&bad_dir);
            Ok::<(), String>(())
        })
    }
}
