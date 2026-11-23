use super::{tests::finish, *};
use crate::{
    bootstrap::init_globals,
    config::ResolvedDomainConfig,
    permission::PermissionSubjectIndex,
    server::{WorldEntry, test_server_with_worlds_and_config},
    test_support::{TestPlayerBuilder, fresh_test_world, test_runtime_config},
};
use std::slice::from_ref;
use tokio::runtime::Builder;

#[test]
fn retiring_worlds_refuse_every_entry() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("retiring_home");
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
        let template_key = Identifier::new_static("retiring_test", "template");
        server.register_ready_template(super::clone_tests::chest_template(
            template_key.clone(),
            Identifier::from_steel("empty"),
        ));
        let key = Identifier::new(home.domain().to_owned(), "the_nether".to_owned());
        let id = server.clone_world(
            &template_key,
            key.clone(),
            DomainTarget::Existing,
            CloneOverrides {
                spawn: Some(BlockPos::new(0, 80, 0)),
                ..CloneOverrides::default()
            },
        )?;
        finish(&server, id).await?;
        let clone = server.worlds.get(&key).ok_or("clone not published")?;

        clone.begin_retiring();

        let player = TestPlayerBuilder::new(Arc::clone(home), "Steve", 1).build();
        let error = server
            .queue_player_world_selection(player, &clone, WorldEntry::Usual)
            .expect_err("world selection must refuse a retiring world");
        assert!(error.contains("being removed"), "{error}");
        assert!(server.worlds.resolve_nether_portal_target(home).is_none());
        assert!(
            server
                .resolve_saved_world(&key.to_string(), home.domain(), None, "Steve")
                .is_none()
        );
        Ok(())
    })
}
