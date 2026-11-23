//! Platform-selected native libraries from extension ZIP packages.

use serde::Deserialize;
use std::{
    collections::BTreeMap,
    env::consts::{ARCH, OS},
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};
use steel_registry::client_content::ResourcePackAssets;
use tempfile::TempDir;
use zip::ZipArchive;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    package: Package,
    targets: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    name: String,
    version: String,
}

pub(super) struct PreparedPackage {
    pub name: String,
    pub library: PathBuf,
    pub directory: TempDir,
    pub assets: BTreeMap<String, Vec<u8>>,
}

pub(super) fn prepare(path: &Path) -> Result<PreparedPackage, String> {
    let file = File::open(path)
        .map_err(|e| format!("cannot open extension archive {}: {e}", path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("invalid extension ZIP: {e}"))?;
    let mut source = String::new();
    archive
        .by_name("extension.toml")
        .map_err(|e| format!("missing extension.toml: {e}"))?
        .read_to_string(&mut source)
        .map_err(|e| format!("cannot read extension manifest: {e}"))?;
    let manifest: Manifest =
        toml::from_str(&source).map_err(|e| format!("invalid extension manifest: {e}"))?;
    if manifest.package.name.is_empty() || manifest.package.version.is_empty() {
        return Err("extension manifest requires a package name and version".into());
    }
    let target = format!("{OS}-{ARCH}");
    let member = manifest.targets.get(&target).ok_or_else(|| {
        format!(
            "extension {} does not support {target}",
            manifest.package.name
        )
    })?;
    if member.contains('\\')
        || Path::new(member)
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err("extension library must have a relative, enclosed archive path".into());
    }
    let mut library = archive
        .by_name(member)
        .map_err(|e| format!("missing extension library {member}: {e}"))?;
    if library.is_dir()
        || library.enclosed_name().is_none()
        || library
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
    {
        return Err("extension library must be a regular enclosed archive member".into());
    }
    let file_name = Path::new(member)
        .file_name()
        .ok_or("extension library path has no filename")?;
    let directory = tempfile::Builder::new()
        .prefix("steel-extension-")
        .tempdir()
        .map_err(|e| format!("cannot create extension extraction directory: {e}"))?;
    let path = directory.path().join(file_name);
    let mut output = File::create_new(&path)
        .map_err(|e| format!("cannot create extracted extension library: {e}"))?;
    io::copy(&mut library, &mut output)
        .map_err(|e| format!("cannot extract extension library: {e}"))?;
    output
        .sync_all()
        .map_err(|e| format!("cannot flush extension library: {e}"))?;
    drop(library);
    let assets = read_assets(&mut archive)?;
    Ok(PreparedPackage {
        name: manifest.package.name,
        library: path,
        directory,
        assets,
    })
}

fn read_assets(archive: &mut ZipArchive<File>) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut assets = BTreeMap::new();
    let mut remaining = ResourcePackAssets::MAX_BYTES;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        if entry.is_dir() || !entry.name().starts_with("assets/") {
            continue;
        }
        let path = entry.name().to_owned();
        ResourcePackAssets::asset_path(&path)?;
        if entry.is_symlink() || assets.contains_key(&path) {
            return Err(format!(
                "resource pack asset must be a unique regular file: {path}"
            ));
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(remaining + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read resource pack asset {path}: {error}"))?;
        remaining = remaining
            .checked_sub(bytes.len() as u64)
            .ok_or("extension package exceeds the resource pack asset limit")?;
        assets.insert(path, bytes);
    }
    Ok(assets)
}

/// Lists archive paths in stable order. A missing extension directory is empty.
pub fn discover_archives(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot scan {}: {error}", directory.display())),
    };
    let mut archives = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read extension directory entry: {e}"))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "zip")
            && entry.file_type().map_err(|e| e.to_string())?.is_file()
        {
            archives.push(path);
        }
    }
    archives.sort();
    Ok(archives)
}

#[cfg(test)]
mod tests;
