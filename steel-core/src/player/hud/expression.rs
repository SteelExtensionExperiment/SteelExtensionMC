//! Data references and bounded bar formatting. Substitution operates on JSON
//! string values, so player data cannot inject component properties.

use super::catalog::{self, SourceKey};
use crate::entity::custom_data::{CustomData, DataPath};
use simdnbt::owned::NbtTag;
use std::collections::BTreeMap;
use text_components::TextComponent;

pub(super) struct RenderContext<'a> {
    pub data: &'a CustomData,
    pub owner: Option<&'a SourceKey>,
    pub overrides: &'a BTreeMap<String, String>,
}

impl RenderContext<'_> {
    fn path(&self, key: &str) -> Result<String, String> {
        if key.contains(':') {
            DataPath::parse(key)?;
            return Ok(key.to_owned());
        }
        if let Some(path) = self.overrides.get(key) {
            DataPath::parse(path)?;
            return Ok(path.clone());
        }
        let aliases = catalog::aliases();
        let matches = aliases
            .get(key)
            .ok_or_else(|| format!("unknown HUD alias {key}; configure server.hud.aliases"))?;
        if let Some(owner) = self.owner
            && let Some((_, path)) = matches.iter().find(|(source, _)| source == owner)
        {
            return Ok(path.clone());
        }
        if matches.len() != 1 {
            let owners = matches
                .iter()
                .map(|(owner, _)| owner.0.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "HUD alias {key} has multiple providers [{owners}]; select server.hud.aliases.{key}"
            ));
        }
        Ok(matches[0].1.clone())
    }

    pub fn value(&self, key: &str) -> Result<Option<&NbtTag>, String> {
        let path = DataPath::parse(&self.path(key.trim())?)?;
        Ok(self.data.get(&path))
    }

    pub fn ratio(&self, key: &str) -> Result<Option<f32>, String> {
        let Some(tag) = self.value(key)? else {
            return Ok(None);
        };
        let gauge = tag
            .compound()
            .ok_or("bar needs a gauge compound with current and maximum")?;
        let current = gauge
            .get("current")
            .and_then(CustomData::number)
            .ok_or("gauge current is not numeric")?;
        let maximum = gauge
            .get("maximum")
            .and_then(CustomData::number)
            .ok_or("gauge maximum is not numeric")?;
        if !current.is_finite() || !maximum.is_finite() || maximum <= 0.0 {
            return Err("invalid gauge bounds".into());
        }
        Ok(Some((current / maximum).clamp(0.0, 1.0) as f32))
    }

    fn expression(&self, expression: &str) -> Result<Option<String>, String> {
        let expression = expression.trim();
        if let Some(args) = expression
            .strip_prefix("bar(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let (key, width) = args
                .rsplit_once(',')
                .ok_or("bar expects a data reference and width")?;
            let width = width.trim().parse::<usize>().map_err(|e| e.to_string())?;
            if !(1..=256).contains(&width) {
                return Err("bar width must be 1–256".into());
            }
            let Some(ratio) = self.ratio(key.trim())? else {
                return Ok(None);
            };
            let filled = (ratio * f32::from(width as u16)).floor() as usize;
            return Ok(Some("█".repeat(filled) + &"░".repeat(width - filled)));
        }
        let Some(value) = self.value(expression)? else {
            return Ok(None);
        };
        if let Some(number) = CustomData::number(value) {
            return Ok(Some(number.to_string()));
        }
        if let Some(text) = value.string() {
            return Ok(Some(text.to_string()));
        }
        if let Some(gauge) = value.compound()
            && let Some(current) = gauge.get("current").and_then(CustomData::number)
        {
            return Ok(Some(current.to_string()));
        }
        Err("HUD references must select a scalar or gauge".into())
    }

    fn string(&self, template: &str) -> Result<Option<String>, String> {
        let mut out = String::new();
        let mut rest = template;
        while let Some(index) = rest.find("${") {
            out.push_str(&rest[..index]);
            rest = &rest[index + 2..];
            let end = rest.find('}').ok_or("unclosed HUD expression")?;
            let Some(value) = self.expression(&rest[..end])? else {
                return Ok(None);
            };
            out.push_str(&value);
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        Ok(Some(out))
    }

    fn json(&self, value: &mut serde_json::Value) -> Result<bool, String> {
        match value {
            serde_json::Value::String(template) => {
                let Some(rendered) = self.string(template)? else {
                    return Ok(false);
                };
                *template = rendered;
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    if !self.json(value)? {
                        return Ok(false);
                    }
                }
            }
            serde_json::Value::Object(fields) => {
                for value in fields.values_mut() {
                    if !self.json(value)? {
                        return Ok(false);
                    }
                }
            }
            _ => {}
        }
        Ok(true)
    }

    pub fn text(&self, template: &str) -> Result<Option<TextComponent>, String> {
        let trimmed = template.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            let mut json: serde_json::Value =
                serde_json::from_str(trimmed).map_err(|e| e.to_string())?;
            if !self.json(&mut json)? {
                return Ok(None);
            }
            return serde_json::from_value(json)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        Ok(self.string(template)?.map(TextComponent::plain))
    }
}
