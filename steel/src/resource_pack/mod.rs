//! Builds a deterministic resource pack from extension assets and administrator ZIPs.

mod hosting;
use hosting::ResourcePackHost;

use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Cursor, Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use hyper::body::Bytes;
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use steel_protocol::packets::common::{CResourcePackPush, MAX_RESOURCE_PACK_URL_LENGTH};
use steel_registry::client_content::ResourcePackAssets;
use steel_utils::version::RESOURCE_PACK_FORMAT;
use tokio::net::TcpListener;
use uuid::Builder;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const ITEM_ATLAS_PATH: &str = "assets/minecraft/atlases/items.json";

/// Controls whether joining clients are offered the combined pack.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourcePackMode {
    /// Do not generate or offer a pack.
    #[default]
    Disabled,
    /// Offer a pack and allow clients to use fallback visuals.
    Optional,
    /// Require the pack to finish applying before the player can join.
    Required,
}

/// Configuration for generating and distributing one resource pack.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourcePackConfig {
    /// Whether the combined pack is offered and required.
    pub mode: ResourcePackMode,
    /// Where Steel atomically writes the combined ZIP.
    pub output: PathBuf,
    /// The public HTTP(S) URL clients will download.
    pub url: Option<String>,
    /// When set, Steel also serves the pack from this socket.
    pub bind: Option<SocketAddr>,
    /// Additional packs whose assets will be included.
    pub include_zips: Vec<PathBuf>,
}

impl Default for ResourcePackConfig {
    fn default() -> Self {
        Self {
            mode: ResourcePackMode::Disabled,
            output: "resource-pack.zip".into(),
            url: None,
            bind: None,
            include_zips: Vec::new(),
        }
    }
}

pub(crate) struct ResourcePack {
    pub offer: Arc<CResourcePackPush>,
    pub host: Option<ResourcePackHost>,
}

impl ResourcePackConfig {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        self.download_url().map(|_| ())
    }

    fn download_url(&self) -> Result<Option<Url>, &'static str> {
        if self.mode == ResourcePackMode::Disabled {
            return Ok(None);
        }
        let raw = self
            .url
            .as_ref()
            .ok_or("resource_pack.url is required when a pack is enabled")?;
        let url = Url::parse(raw).map_err(|_| "resource_pack.url must be a valid HTTP(S) URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.as_str().len() > MAX_RESOURCE_PACK_URL_LENGTH
        {
            return Err(
                "resource_pack.url must be a public HTTP(S) URL without credentials or a fragment",
            );
        }
        if self.output.file_name().is_none() {
            return Err("resource_pack.output must name a ZIP file");
        }
        Ok(Some(url))
    }

    pub(crate) async fn prepare(
        self,
        assets: &'static ResourcePackAssets,
    ) -> Result<Option<ResourcePack>, String> {
        let Some(url) = self.download_url()? else {
            return Ok(None);
        };
        let bind = self.bind;
        let required = self.mode == ResourcePackMode::Required;
        let listener = if let Some(bind) = bind {
            Some(TcpListener::bind(bind).await.map_err(|error| {
                format!("cannot bind resource pack listener to {bind}: {error}")
            })?)
        } else {
            None
        };
        let bytes = tokio::task::spawn_blocking(move || {
            if self.output.exists() {
                let output = self
                    .output
                    .canonicalize()
                    .map_err(|error| format!("cannot resolve resource pack output: {error}"))?;
                for input in &self.include_zips {
                    let input = input.canonicalize().map_err(|error| {
                        format!("cannot resolve resource pack {}: {error}", input.display())
                    })?;
                    if input == output {
                        return Err("the resource pack output cannot also be an input".into());
                    }
                }
            }
            let bytes = build_zip(assets, &self.include_zips)?;
            write_zip(&self.output, &bytes).map_err(|error| {
                format!(
                    "cannot write resource pack {}: {error}",
                    self.output.display()
                )
            })?;
            log::info!("Wrote combined resource pack to {}", self.output.display());
            Ok::<_, String>(bytes)
        })
        .await
        .map_err(|error| format!("resource pack build task failed: {error}"))??;
        let hash = Sha1::digest(&bytes);
        let mut id = [0; 16];
        id.copy_from_slice(&hash[..16]);
        let offer = Arc::new(CResourcePackPush {
            id: Builder::from_sha1_bytes(id).into_uuid(),
            url: url.to_string(),
            hash: hex::encode(hash),
            required,
            prompt: None,
        });
        Ok(Some(ResourcePack {
            offer,
            host: listener.map(|listener| ResourcePackHost {
                listener,
                path: url.path().to_owned().into(),
                bytes: Bytes::from(bytes),
            }),
        }))
    }
}

fn build_zip(extensions: &ResourcePackAssets, extra_packs: &[PathBuf]) -> Result<Vec<u8>, String> {
    let mut assets = BTreeMap::new();
    let mut remaining = ResourcePackAssets::MAX_BYTES;
    for path in extra_packs {
        let file = File::open(path)
            .map_err(|error| format!("cannot open resource pack {}: {error}", path.display()))?;
        add_zip(file, &mut assets, &mut remaining)
            .map_err(|error| format!("cannot merge resource pack {}: {error}", path.display()))?;
    }
    for (path, bytes) in extensions.files() {
        spend_asset_bytes(&mut remaining, bytes.len() as u64)?;
        insert_asset(&mut assets, path.to_owned(), bytes.to_vec())?;
    }
    add_ui_atlas_source(&mut assets, &mut remaining)?;
    encode_zip(assets).map_err(|error| format!("cannot encode resource pack: {error}"))
}

fn add_ui_atlas_source(
    assets: &mut BTreeMap<String, Vec<u8>>,
    remaining: &mut u64,
) -> Result<(), String> {
    if !assets.keys().any(|path| {
        path.strip_prefix("assets/")
            .and_then(|path| path.split_once('/'))
            .is_some_and(|(_, file)| file.starts_with("textures/ui/") && file.ends_with(".png"))
    }) {
        return Ok(());
    }

    // Vanilla reads the atlas sources from the whole pack stack, so we only add the UI directory.
    let existing = assets.get(ITEM_ATLAS_PATH);
    let mut atlas = match existing {
        Some(bytes) => serde_json::from_slice(bytes)
            .map_err(|error| format!("cannot read item atlas {ITEM_ATLAS_PATH}: {error}"))?,
        None => json!({ "sources": [] }),
    };
    let sources = atlas
        .get_mut("sources")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("item atlas {ITEM_ATLAS_PATH} must contain a sources array"))?;
    let source = json!({ "type": "minecraft:directory", "source": "ui", "prefix": "ui/" });
    if sources.contains(&source) {
        return Ok(());
    }
    sources.push(source);
    let bytes = serde_json::to_vec(&atlas)
        .map_err(|error| format!("cannot encode item atlas {ITEM_ATLAS_PATH}: {error}"))?;
    let previous_size = existing.map_or(0, Vec::len);
    spend_asset_bytes(remaining, bytes.len().saturating_sub(previous_size) as u64)?;
    assets.insert(ITEM_ATLAS_PATH.to_owned(), bytes);
    Ok(())
}

fn spend_asset_bytes(remaining: &mut u64, size: u64) -> Result<(), String> {
    *remaining = remaining
        .checked_sub(size)
        .ok_or("resource pack assets exceed the 250 MiB limit")?;
    Ok(())
}

fn add_zip(
    input: impl Read + io::Seek,
    assets: &mut BTreeMap<String, Vec<u8>>,
    remaining: &mut u64,
) -> Result<(), String> {
    let mut archive = ZipArchive::new(input).map_err(|error| error.to_string())?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        if entry.is_dir() || !entry.name().starts_with("assets/") {
            continue;
        }
        let path = entry.name().to_owned();
        ResourcePackAssets::asset_path(&path)?;
        if entry.is_symlink() {
            return Err(format!("resource pack asset is a symbolic link: {path}"));
        }
        let max = *remaining;
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        spend_asset_bytes(remaining, bytes.len() as u64)?;
        insert_asset(assets, path, bytes)?;
    }
    Ok(())
}

fn insert_asset(
    assets: &mut BTreeMap<String, Vec<u8>>,
    path: String,
    bytes: Vec<u8>,
) -> Result<(), String> {
    use std::collections::btree_map::Entry;
    match assets.entry(path) {
        Entry::Occupied(existing) if existing.get() != &bytes => Err(format!(
            "conflicting resource pack asset {}",
            existing.key()
        )),
        Entry::Occupied(_) => Ok(()),
        Entry::Vacant(entry) => {
            entry.insert(bytes);
            Ok(())
        }
    }
}

fn encode_zip(assets: BTreeMap<String, Vec<u8>>) -> io::Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("pack.mcmeta", options)?;
    serde_json::to_writer(
        &mut zip,
        &serde_json::json!({
            "pack": {
                "description": "Steel server resources",
                "min_format": RESOURCE_PACK_FORMAT,
                "max_format": RESOURCE_PACK_FORMAT,
            }
        }),
    )?;
    for (path, bytes) in assets {
        zip.start_file(path, options)?;
        zip.write_all(&bytes)?;
    }
    let bytes = zip.finish()?.into_inner();
    if bytes.len() as u64 > ResourcePackAssets::MAX_BYTES {
        return Err(io::Error::other(
            "resource pack ZIP exceeds the client download limit",
        ));
    }
    Ok(bytes)
}

fn write_zip(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o644))?;
    }
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
