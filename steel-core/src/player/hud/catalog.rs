//! Process-lifetime identities and default data bindings of installed extensions.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};
use steel_utils::locks::SyncRwLock;

use crate::entity::custom_data::DataPath;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// Full owner and local source key used in owner configuration.
pub struct SourceKey(pub String);

#[derive(Debug, Clone)]
/// Identity and default data bindings of one authenticated producer.
pub struct HudSource {
    /// Full key visible in owner configuration and diagnostics.
    pub key: SourceKey,
    bindings: Arc<SyncRwLock<BTreeMap<String, String>>>,
}

static SOURCES: LazyLock<SyncRwLock<Vec<Arc<HudSource>>>> =
    LazyLock::new(|| SyncRwLock::new(Vec::new()));
static REVISION: AtomicU64 = AtomicU64::new(1);
pub(super) fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

/// Registers a source within an authenticated extension owner; repeat registration is idempotent.
pub fn register(owner: &str, key: &str) -> Result<u64, String> {
    if owner == "steel"
        || key.is_empty()
        || key.contains(':')
        || key.chars().any(char::is_whitespace)
    {
        return Err("HUD source needs a nonempty local key and an extension owner".into());
    }
    let full = SourceKey(format!("{owner}:{key}"));
    let mut sources = SOURCES.write();
    if let Some(index) = sources.iter().position(|source| source.key == full) {
        return Ok(index as u64);
    }
    let id = sources.len() as u64;
    sources.push(Arc::new(HudSource {
        key: full,
        bindings: Arc::new(SyncRwLock::new(BTreeMap::new())),
    }));
    Ok(id)
}

/// Resolves an opaque API source handle.
pub fn source(id: u64) -> Result<Arc<HudSource>, String> {
    let index = usize::try_from(id).map_err(|e| e.to_string())?;
    SOURCES
        .read()
        .get(index)
        .map(Arc::clone)
        .ok_or_else(|| "unknown HUD source".into())
}

/// Registers or replaces only this producer's default data alias.
pub fn bind(id: u64, alias: &str, path: &str) -> Result<(), String> {
    if alias.is_empty() || !alias.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("invalid HUD data alias".into());
    }
    DataPath::parse(path)?;
    source(id)?
        .bindings
        .write()
        .insert(alias.to_owned(), path.to_owned());
    REVISION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

pub(super) fn aliases() -> BTreeMap<String, Vec<(SourceKey, String)>> {
    let mut aliases: BTreeMap<String, Vec<(SourceKey, String)>> = BTreeMap::new();
    for source in SOURCES.read().iter() {
        for (alias, path) in source.bindings.read().iter() {
            aliases
                .entry(alias.clone())
                .or_default()
                .push((source.key.clone(), path.clone()));
        }
    }
    aliases
}
