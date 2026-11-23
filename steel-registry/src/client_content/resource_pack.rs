use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};

use steel_utils::Identifier;

use crate::{items::ItemRef, resolvable_profile::ResolvableProfile, vanilla_items};

/// A single head for inventory and cube displays when the client has no pack.
#[derive(PartialEq, Eq)]
pub struct PackHeadFallback {
    /// Vanilla head item using the standard cubical skull model.
    pub item: ItemRef,
    /// Optional static skin for a player head.
    pub profile: Option<ResolvableProfile>,
}

impl PackHeadFallback {
    /// A player head with a supplied skin.
    #[must_use]
    pub fn player_head(profile: ResolvableProfile) -> Self {
        Self {
            item: &vanilla_items::PLAYER_HEAD,
            profile: Some(profile),
        }
    }
}

/// Startup-registered assets for the combined server resource pack.
#[derive(Default)]
pub struct ResourcePackAssets {
    files: BTreeMap<String, Vec<u8>>,
    item_models: BTreeMap<String, BTreeSet<String>>,
    head_fallbacks: BTreeMap<Identifier, PackHeadFallback>,
}

impl ResourcePackAssets {
    /// Maximum size of a vanilla client download. Also bounds ZIP asset expansion.
    pub const MAX_BYTES: u64 = 250 * 1024 * 1024;

    /// Adds vanilla-layout asset files. A batch is accepted in full or rejected.
    pub(crate) fn register(&mut self, files: BTreeMap<String, Vec<u8>>) -> Result<(), String> {
        self.validate_files(&files)?;
        self.insert_validated_files(files);
        Ok(())
    }

    pub(super) fn validate_files(&self, files: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
        for (path, bytes) in files {
            let (namespace, _) = Self::asset_path(path)?;
            if namespace == Identifier::VANILLA_NAMESPACE {
                return Err(format!(
                    "extension pack assets must use a custom namespace: {path}"
                ));
            }
            if self
                .files
                .get(path)
                .is_some_and(|existing| existing != bytes)
            {
                return Err(format!("conflicting extension pack asset {path}"));
            }
        }
        Ok(())
    }

    pub(super) fn insert_validated_files(&mut self, files: BTreeMap<String, Vec<u8>>) {
        for (path, data) in files {
            let Ok((namespace, relative)) = Self::asset_path(&path) else {
                unreachable!("validated resource pack path is invalid");
            };
            if let Some(model) = relative
                .strip_prefix("items/")
                .and_then(|p| p.strip_suffix(".json"))
            {
                self.item_models
                    .entry(namespace.to_owned())
                    .or_default()
                    .insert(model.to_owned());
            }
            self.files.insert(path, data);
        }
    }

    /// Returns the namespace and namespace-relative path of an enclosed asset.
    pub fn asset_path(path: &str) -> Result<(&str, &str), String> {
        let (namespace, relative) = path
            .strip_prefix("assets/")
            .and_then(|path| path.split_once('/'))
            .ok_or_else(|| format!("invalid resource pack asset path {path:?}"))?;
        if matches!(namespace, "" | "." | "..")
            || !Identifier::validate_namespace(namespace)
            || !Identifier::validate_path(relative)
            || relative
                .split('/')
                .any(|part| matches!(part, "" | "." | ".."))
        {
            return Err(format!("invalid resource pack asset path {path:?}"));
        }
        Ok((namespace, relative))
    }

    /// Iterates assets in archive order.
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
    }

    pub(super) fn has_item_models(&self) -> bool {
        !self.item_models.is_empty()
    }

    pub(super) fn has_item_model(&self, model: &Identifier) -> bool {
        self.item_models
            .get(model.namespace.as_ref())
            .is_some_and(|paths| paths.contains(model.path.as_ref()))
    }

    pub(super) fn register_head_fallback(
        &mut self,
        model: Identifier,
        fallback: PackHeadFallback,
    ) -> Result<(), String> {
        if !self.has_item_model(&model) {
            return Err(format!(
                "no contributed item model {model} for the head fallback"
            ));
        }
        self.validate_head_fallback(&model, &fallback)?;
        self.insert_head_fallback(model, fallback);
        Ok(())
    }

    pub(super) fn validate_head_fallback(
        &self,
        model: &Identifier,
        fallback: &PackHeadFallback,
    ) -> Result<(), String> {
        if ![
            &*vanilla_items::PLAYER_HEAD,
            &*vanilla_items::SKELETON_SKULL,
            &*vanilla_items::WITHER_SKELETON_SKULL,
            &*vanilla_items::ZOMBIE_HEAD,
            &*vanilla_items::CREEPER_HEAD,
        ]
        .iter()
        .any(|head| std::ptr::eq(*head, fallback.item))
        {
            return Err("a head fallback requires a cubical vanilla head item".into());
        }
        if fallback.profile.is_some() && !std::ptr::eq(fallback.item, &*vanilla_items::PLAYER_HEAD)
        {
            return Err("a fallback skin requires a player head".into());
        }
        if self
            .head_fallbacks
            .get(model)
            .is_some_and(|existing| existing != fallback)
        {
            return Err(format!("conflicting head fallback for {model}"));
        }
        Ok(())
    }

    pub(super) fn insert_head_fallback(&mut self, model: Identifier, fallback: PackHeadFallback) {
        self.head_fallbacks.insert(model, fallback);
    }

    pub(super) fn head_fallback(&self, model: &Identifier) -> Option<&PackHeadFallback> {
        self.head_fallbacks.get(model)
    }
}

#[derive(Clone, Copy)]
struct PackProjection {
    include_models: bool,
    encountered_model: bool,
}

thread_local! {
    static PACK_PROJECTION: Cell<PackProjection> = const { Cell::new(PackProjection {
        include_models: false,
        encountered_model: false,
    }) };
}

struct ProjectionGuard(PackProjection);

impl Drop for ProjectionGuard {
    fn drop(&mut self) {
        PACK_PROJECTION.set(self.0);
    }
}

/// Selects the pack representation for a synchronous packet write. The flag reports
/// whether encoding another representation would change the packet.
pub fn project_pack_models<T>(include_models: bool, write_packet: impl FnOnce() -> T) -> (T, bool) {
    let _guard = ProjectionGuard(PACK_PROJECTION.replace(PackProjection {
        include_models,
        encountered_model: false,
    }));
    let result = write_packet();
    (result, PACK_PROJECTION.get().encountered_model)
}

pub(super) fn use_pack_model() -> bool {
    let mut projection = PACK_PROJECTION.get();
    projection.encountered_model = true;
    PACK_PROJECTION.set(projection);
    projection.include_models
}
