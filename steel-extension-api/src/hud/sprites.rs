//! Bitmap font generation for named HUD artwork.

use std::{collections::BTreeSet, fmt::Write};

use crate::{
    content::ContentRegistry,
    text::{Text, write_string},
};

// Matches the glyphs emitted by the HUD bar() expression.
const FILLED_SEGMENT: &str = "█";
const EMPTY_SEGMENT: &str = "░";
const FIRST_PRIVATE_USE: u32 = 0xe000;
const LAST_PRIVATE_USE: u32 = 0xf8ff;
const DEFAULT_GLYPH_HEIGHT: i32 = 8;
const DEFAULT_GLYPH_ASCENT: i32 = 7;

/// Named artwork loaded from a PNG texture in the client resource pack.
#[derive(Debug, Clone, Copy)]
pub struct HudIcon<'a> {
    name: &'a str,
    texture: &'a str,
}

impl<'a> HudIcon<'a> {
    /// Declares an icon by name and its namespaced texture path, including `.png`.
    #[must_use]
    pub const fn new(name: &'a str, texture: &'a str) -> Self {
        Self { name, texture }
    }
}

#[derive(Debug, Clone, Copy)]
enum GaugeArt<'a> {
    Strip(&'a str),
    Pair { filled: &'a str, empty: &'a str },
}

/// A set of icons and an optional gauge compiled into one namespaced bitmap font.
#[derive(Debug, Clone, Copy)]
pub struct HudSprites<'a> {
    key: &'a str,
    gauge: Option<GaugeArt<'a>>,
    icons: &'a [HudIcon<'a>],
    height: i32,
    ascent: i32,
}

impl<'a> HudSprites<'a> {
    /// Declares a sprite set with a namespaced key and vanilla text line metrics.
    #[must_use]
    pub const fn new(key: &'a str) -> Self {
        Self {
            key,
            gauge: None,
            icons: &[],
            height: DEFAULT_GLYPH_HEIGHT,
            ascent: DEFAULT_GLYPH_ASCENT,
        }
    }

    /// Uses a PNG with two equal-width cells, filled followed by empty.
    #[must_use]
    pub const fn with_gauge_strip(mut self, texture: &'a str) -> Self {
        self.gauge = Some(GaugeArt::Strip(texture));
        self
    }

    /// Uses two individual textures for the filled and empty segments.
    #[must_use]
    pub const fn with_gauge_sprites(mut self, filled: &'a str, empty: &'a str) -> Self {
        self.gauge = Some(GaugeArt::Pair { filled, empty });
        self
    }

    /// Declares the icons to include in this set.
    #[must_use]
    pub const fn with_icons(mut self, icons: &'a [HudIcon<'a>]) -> Self {
        self.icons = icons;
        self
    }

    /// Sets the sprite height and ascent in GUI pixels.
    #[must_use]
    pub const fn with_metrics(mut self, height: i32, ascent: i32) -> Self {
        self.height = height;
        self.ascent = ascent;
        self
    }

    /// Generates the font. Texture PNGs can come from the extension's assets or vanilla.
    pub fn register(&self, registry: &mut ContentRegistry<'_>) -> Result<(), String> {
        let (namespace, path) = self
            .key
            .split_once(':')
            .ok_or_else(|| format!("HUD sprite set {} requires a namespace", self.key))?;
        let font = self.font_json()?;
        registry
            .add_resource_asset(
                format!("assets/{namespace}/font/{path}.json")
                    .as_str()
                    .into(),
                font.as_bytes().into(),
            )
            .match_owned(Ok, |error| Err(error.to_string()))
    }

    /// Resolves a named icon to a styled text component for the packed HUD.
    pub fn icon(&self, name: &str) -> Result<Text, String> {
        let index = self
            .icons
            .iter()
            .position(|icon| icon.name == name)
            .ok_or_else(|| format!("HUD sprite set {} has no icon {name:?}", self.key))?;
        Ok(Text::new(icon_glyph(index)?.to_string())
            .font(self.key)
            .color("white"))
    }

    /// Creates both renderers for a gauge driven by a HUD alias or data path.
    pub fn gauge(&self, reference: &str, segments: u16) -> Result<HudGauge<'a>, String> {
        if self.gauge.is_none() {
            return Err(format!("HUD sprite set {} has no gauge", self.key));
        }
        Ok(HudGauge {
            key: self.key,
            expression: format!("${{bar({reference}, {segments})}}"),
        })
    }

    fn font_json(&self) -> Result<String, String> {
        if self.height <= 0 || self.ascent > self.height {
            return Err(format!("invalid HUD sprite metrics for {}", self.key));
        }
        let mut providers = Vec::new();
        match self.gauge {
            Some(GaugeArt::Strip(texture)) => {
                let cells = format!("{FILLED_SEGMENT}{EMPTY_SEGMENT}");
                providers.push(self.bitmap(texture, &cells)?);
            }
            Some(GaugeArt::Pair { filled, empty }) => {
                providers.push(self.bitmap(filled, FILLED_SEGMENT)?);
                providers.push(self.bitmap(empty, EMPTY_SEGMENT)?);
            }
            None => {}
        }
        let mut names = BTreeSet::new();
        for (index, icon) in self.icons.iter().enumerate() {
            if icon.name.is_empty() || !names.insert(icon.name) {
                return Err(format!(
                    "HUD sprite set {} has an empty or duplicate icon name {:?}",
                    self.key, icon.name
                ));
            }
            providers.push(self.bitmap(icon.texture, &icon_glyph(index)?.to_string())?);
        }
        if providers.is_empty() {
            return Err(format!("HUD sprite set {} has no artwork", self.key));
        }
        Ok(format!(r#"{{"providers":[{}]}}"#, providers.join(",")))
    }

    fn bitmap(&self, texture: &str, glyphs: &str) -> Result<String, String> {
        if !is_png_reference(texture) {
            return Err(format!("HUD sprite requires a namespaced PNG: {texture:?}"));
        }
        let mut json = String::from(r#"{"type":"bitmap","file":"#);
        write_string(&mut json, texture);
        let _ = write!(
            json,
            r#","height":{},"ascent":{},"chars":["#,
            self.height, self.ascent
        );
        write_string(&mut json, glyphs);
        json.push_str("]}");
        Ok(json)
    }
}

fn is_png_reference(texture: &str) -> bool {
    let Some((namespace, path)) = texture.split_once(':') else {
        return false;
    };
    let is_segment = |segment: &str| {
        !matches!(segment, "" | "." | "..")
            && segment.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
            })
    };
    is_segment(namespace) && path.ends_with(".png") && path.split('/').all(is_segment)
}

fn icon_glyph(index: usize) -> Result<char, String> {
    (FIRST_PRIVATE_USE..=LAST_PRIVATE_USE)
        .nth(index)
        .and_then(char::from_u32)
        .ok_or_else(|| "HUD sprite set has exhausted its private-use glyphs".into())
}

/// A meter template that can be rendered as vanilla text or textured sprites.
pub struct HudGauge<'a> {
    key: &'a str,
    expression: String,
}

impl HudGauge<'_> {
    /// Draws the meter as ordinary filled and empty block characters.
    #[must_use]
    pub fn vanilla(&self) -> Text {
        Text::new(self.expression.as_str())
    }

    /// Draws the same meter with the registered artwork.
    #[must_use]
    pub fn packed(&self) -> Text {
        self.vanilla().font(self.key).color("white")
    }
}
