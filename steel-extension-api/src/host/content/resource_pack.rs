//! Extension resource assets and common vanilla model definitions.

use std::collections::BTreeMap;

use base64::{Engine as _, prelude::BASE64_STANDARD};
use serde_json::json;
use sha1::{Digest as _, Sha1};
use steel_registry::{
    Registry, RegistryExt,
    blocks::BlockRef,
    client_content::PackHeadFallback,
    game_profile::ProfileProperty,
    resolvable_profile::{PartialProfile, PlayerSkinPatch, ResolvableProfile},
};
use steel_utils::Identifier;
use uuid::Builder;

use crate::content::{BlockItemDefinition, TextureFallback};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

pub(crate) fn add_asset(registry: &mut Registry, path: &str, bytes: &[u8]) -> Result<(), String> {
    registry.register_pack_assets(BTreeMap::from([(path.to_owned(), bytes.to_vec())]))
}

pub(crate) fn register_texture(
    registry: &mut Registry,
    key: &str,
    bytes: &[u8],
    cube: bool,
) -> Result<(), String> {
    let model = key.parse::<Identifier>().map_err(ToString::to_string)?;
    registry.register_pack_assets(texture_files(&model, bytes, cube)?)
}

pub(crate) fn register_textured_cube(
    registry: &mut Registry,
    definition: BlockItemDefinition,
    png: &[u8],
    fallback: TextureFallback,
) -> Result<BlockRef, String> {
    let model = definition
        .block
        .key
        .as_str()
        .parse::<Identifier>()
        .map_err(ToString::to_string)?;
    let files = texture_files(&model, png, true)?;
    let fallback = match fallback {
        TextureFallback::Vanilla => None,
        TextureFallback::Head { skin } => Some(head_fallback(registry, skin.as_str())?),
    };
    let (block, placing) =
        super::prepare_block_item(registry, definition.with_cube_model(&model.to_string()))?;
    registry.register_textured_block_definition(block, placing, files, fallback)
}

fn texture_files(
    model: &Identifier,
    bytes: &[u8],
    cube: bool,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err(format!("texture {model} is not a PNG"));
    }
    let category = if cube { "block" } else { "item" };
    let model_path = format!("{category}/{}", model.path);
    let sprite_path = if !cube && model.path.starts_with("ui/") {
        model.path.as_ref()
    } else {
        &model_path
    };
    let texture = format!("{}:{sprite_path}", model.namespace);
    let model_data = if cube {
        json!({"parent": "minecraft:block/cube_all", "textures": {"all": texture}})
    } else {
        json!({"parent": "minecraft:item/generated", "textures": {"layer0": texture}})
    };
    Ok(BTreeMap::from([
        (
            format!("assets/{}/textures/{sprite_path}.png", model.namespace),
            bytes.to_vec(),
        ),
        (
            format!("assets/{}/models/{model_path}.json", model.namespace),
            model_data.to_string().into_bytes(),
        ),
        (
            format!("assets/{}/items/{}.json", model.namespace, model.path),
            json!({"model": {"type": "minecraft:model", "model": format!("{}:{model_path}", model.namespace)}})
                .to_string()
                .into_bytes(),
        ),
    ]))
}

pub(crate) fn register_head_fallback(
    registry: &mut Registry,
    model: &str,
    skin: &str,
) -> Result<(), String> {
    let model = model.parse::<Identifier>().map_err(ToString::to_string)?;
    let fallback = head_fallback(registry, skin)?;
    registry.register_pack_head_fallback(model, fallback)
}

fn head_fallback(registry: &Registry, skin: &str) -> Result<PackHeadFallback, String> {
    if skin.starts_with("minecraft:") {
        let key = skin.parse::<Identifier>().map_err(ToString::to_string)?;
        let item = registry
            .items
            .by_key(&key)
            .ok_or_else(|| format!("unknown vanilla head {skin}"))?;
        return Ok(PackHeadFallback {
            item,
            profile: None,
        });
    }
    Ok(PackHeadFallback::player_head(skin_profile(skin)?))
}

fn skin_profile(skin: &str) -> Result<ResolvableProfile, String> {
    let skin_hash = skin
        .strip_prefix("https://textures.minecraft.net/texture/")
        .or_else(|| skin.strip_prefix("http://textures.minecraft.net/texture/"))
        .unwrap_or(skin);
    if skin_hash.is_empty()
        || skin_hash.len() > 64
        || !skin_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("a Minecraft skin hash must contain up to 64 lowercase hex digits".into());
    }
    let url = format!("https://textures.minecraft.net/texture/{skin_hash}");
    let textures = json!({ "textures": { "SKIN": { "url": url } } });
    let property = ProfileProperty::new(
        "textures".into(),
        BASE64_STANDARD.encode(textures.to_string()),
        None,
    )
    .map_err(|error| error.to_string())?;
    let mut id_bytes = [0; 16];
    id_bytes.copy_from_slice(&Sha1::digest(url.as_bytes())[..16]);
    let id = Builder::from_custom_bytes(id_bytes).into_uuid();
    let profile =
        PartialProfile::new(None, Some(id), vec![property]).map_err(|error| error.to_string())?;
    Ok(ResolvableProfile::static_partial(
        profile,
        PlayerSkinPatch::default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // An opaque 8×8 PNG used to exercise the registration API.
    const SMALL_CUBE: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x08, 0x06, 0x00, 0x00, 0x00, 0xc4,
        0x0f, 0xbe, 0x8b, 0x00, 0x00, 0x00, 0x16, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x74,
        0x9f, 0x72, 0xfb, 0x3f, 0x03, 0x1e, 0xc0, 0x84, 0x4f, 0x72, 0xf8, 0x28, 0x00, 0x00, 0xcc,
        0x12, 0x02, 0xc5, 0x8e, 0xc8, 0x97, 0x03, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
        0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn rejected_textured_cube_does_not_reserve_pack_assets() -> Result<(), String> {
        use crate::content::{BlockDefinition, ItemAppearance, ItemDefinition, VanillaHead};

        let mut registry = Registry::new_vanilla();
        let key = "head_test:occupied";
        crate::host::content::register_block(&mut registry, BlockDefinition::new(key))?;
        let definition = BlockItemDefinition {
            block: BlockDefinition::new(key),
            item: ItemDefinition {
                key: key.into(),
                name: "Occupied".into(),
                appearance: ItemAppearance::new("minecraft:stone"),
                max_stack_size: 64,
            },
            drop_self: true,
        };
        let error = register_textured_cube(
            &mut registry,
            definition,
            SMALL_CUBE,
            TextureFallback::vanilla_head(VanillaHead::Creeper),
        )
        .err()
        .ok_or("duplicate block unexpectedly registered")?;
        assert!(error.contains("already registered"));
        assert!(
            registry
                .client_content
                .resource_pack()
                .files()
                .next()
                .is_none()
        );
        Ok(())
    }
}
