//! Native furniture registration and real placement, mounting, and sleeping.
#![cfg(feature = "host")]

#[path = "furniture/bed.rs"]
mod bed;
#[path = "furniture/chair.rs"]
mod chair;
#[path = "lucky_block/fixture.rs"]
mod fixture;
#[path = "furniture/helpers.rs"]
mod helpers;
#[path = "furniture/placement.rs"]
mod placement;
#[path = "furniture/slabs.rs"]
mod slabs;

use std::{
    env::consts::{DLL_PREFIX, DLL_SUFFIX},
    error::Error,
    path::PathBuf,
    sync::Arc,
};
use steel_core::{
    behavior::{BLOCK_BEHAVIORS, BlockBehaviorRegistry},
    bootstrap::init_globals,
    extension::ExtensionHooks,
};
use steel_extension_api::host;
use steel_registry::{REGISTRY, RegistryExt, vanilla_dimension_types};
use steel_utils::{Identifier, locks::SyncMutex};

#[test]
#[ignore = "requires building extension-example/furniture"]
fn furniture_places_sits_sleeps_breaks_and_persists() -> Result<(), Box<dyn Error>> {
    let library = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../extension-example/furniture/target/debug")
        .join(format!("{DLL_PREFIX}steel_furniture_extension{DLL_SUFFIX}"));
    let chair_key = Identifier::new_static("furniture", "chair");
    let observed_behavior = Arc::new(SyncMutex::new(None));
    let capture = Arc::clone(&observed_behavior);
    BlockBehaviorRegistry::on_registration(move |registry| {
        *capture.lock() = REGISTRY
            .blocks
            .by_key(&chair_key)
            .map(|chair| (chair, registry.get_behavior_shared(chair)));
    })?;
    let mut hooks = ExtensionHooks::default();
    // SAFETY: this trusted example uses the checked native v4 contract; the host
    // pins its library and all callback allocations for their required lifetime.
    let extension = unsafe { host::load(&library, "", &mut hooks) }?;
    init_globals();
    let (chair, before_replacements) = observed_behavior
        .lock()
        .take()
        .ok_or("chair was unavailable to behavior replacement hooks")?;
    assert!(Arc::ptr_eq(
        &before_replacements,
        &BLOCK_BEHAVIORS.get_behavior_shared(chair),
    ));
    slabs::check_geometry()?;
    let data_root = tempfile::tempdir()?;
    let _initialized = host::initialize_all(vec![extension], data_root.path(), &mut hooks)?;
    let mut fixture = fixture::Fixture::with_dimension(&vanilla_dimension_types::OVERWORLD, true)?;
    helpers::floor(&fixture);
    fixture.join_player()?;
    placement::check()?;
    slabs::check_shelf_face_click(&fixture)?;
    chair::check(&fixture)?;
    slabs::check(&fixture)?;
    bed::check(&fixture)?;
    bed::check_explosion()?;
    Ok(())
}
