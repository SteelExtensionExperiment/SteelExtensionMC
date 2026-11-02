//! Shared entity custom data. Namespaced extension writes preserve native fields.
//! Mutations run under the entity data lock, without callbacks or I/O.

use simdnbt::owned::{NbtCompound, NbtTag};
use std::sync::atomic::{AtomicU64, Ordering};
use steel_utils::Identifier;

static REVISION: AtomicU64 = AtomicU64::new(1);

/// A namespaced root followed by slash-separated compound fields.
#[derive(Debug)]
pub struct DataPath {
    parts: Vec<String>,
}

impl DataPath {
    /// Validates an extension root and its compound-field path.
    pub fn parse(path: &str) -> Result<Self, String> {
        let parts: Vec<_> = path.split('/').map(str::to_owned).collect();
        let Some(root) = parts.first() else {
            return Err("data path is empty".into());
        };
        let key = root.parse::<Identifier>().map_err(ToString::to_string)?;
        if key.namespace == "minecraft" || !root.contains(':') || parts.iter().any(String::is_empty)
        {
            return Err("custom data needs an extension namespace and nonempty field names".into());
        }
        Ok(Self { parts })
    }
}

#[derive(Debug, Clone, Default)]
/// Native custom data and namespaced extension compounds for one entity.
pub struct CustomData {
    root: NbtCompound,
    revision: u64,
}

impl PartialEq for CustomData {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
    }
}

impl CustomData {
    /// Loads native custom data without restricting its vanilla field names or types.
    #[must_use]
    pub fn from_compound(root: NbtCompound) -> Self {
        Self {
            root,
            revision: REVISION.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Borrows the native compound for serialization without copying it.
    #[must_use]
    pub const fn as_compound(&self) -> &NbtCompound {
        &self.root
    }

    #[must_use]
    /// Creates an owned NBT snapshot for persistence.
    pub fn snapshot(&self) -> NbtCompound {
        self.root.clone()
    }

    #[must_use]
    /// Reads a typed field without changing it.
    pub fn get(&self, path: &DataPath) -> Option<&NbtTag> {
        let mut compound = &self.root;
        let (last, parents) = path.parts.split_last()?;
        for key in parents {
            compound = compound.compound(key)?;
        }
        compound.get(last)
    }

    fn validate_parents(&self, path: &DataPath) -> Result<(), String> {
        let mut compound = &self.root;
        for key in &path.parts[..path.parts.len() - 1] {
            match compound.get(key) {
                Some(NbtTag::Compound(child)) => compound = child,
                Some(_) => return Err(format!("data field {key} is not a compound")),
                None => return Ok(()),
            }
        }
        Ok(())
    }

    /// Replaces one field, creating missing compound parents.
    pub fn set(&mut self, path: &DataPath, value: NbtTag) -> Result<(), String> {
        if path.parts.len() == 1 && value.compound().is_none() {
            return Err("an extension root must be a compound".into());
        }
        self.validate_parents(path)?;
        let mut compound = &mut self.root;
        let Some((last, parents)) = path.parts.split_last() else {
            return Err("data path is empty".into());
        };
        for key in parents {
            if compound.get(key).is_none() {
                compound.insert(key.as_str(), NbtCompound::new());
            }
            compound = compound
                .compound_mut(key)
                .ok_or("validated data parent is not a compound")?;
        }
        if let Some(previous) = compound.get_mut(last) {
            *previous = value;
        } else {
            compound.insert(last.as_str(), value);
        }
        self.revision = REVISION.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Initializes an absent field, preserving an existing value.
    pub fn initialize(&mut self, path: &DataPath, value: NbtTag) -> Result<bool, String> {
        if self.get(path).is_some() {
            return Ok(false);
        }
        self.set(path, value)?;
        Ok(true)
    }

    /// Removes one field, preserving other namespaces and siblings.
    pub fn remove(&mut self, path: &DataPath) -> Option<NbtTag> {
        let (last, parents) = path.parts.split_last()?;
        let mut compound = &mut self.root;
        for key in parents {
            compound = compound.compound_mut(key)?;
        }
        let removed = compound.remove(last);
        if removed.is_some() {
            self.revision = REVISION.fetch_add(1, Ordering::Relaxed);
        }
        removed
    }

    /// Promotes a numeric NBT tag to a double.
    #[must_use]
    pub fn number(value: &NbtTag) -> Option<f64> {
        match value {
            NbtTag::Byte(v) => Some(f64::from(*v)),
            NbtTag::Short(v) => Some(f64::from(*v)),
            NbtTag::Int(v) => Some(f64::from(*v)),
            NbtTag::Long(v) => Some(*v as f64),
            NbtTag::Float(v) => Some(f64::from(*v)),
            NbtTag::Double(v) => Some(*v),
            _ => None,
        }
    }

    /// Checks and subtracts in one lock scope. A failed spend changes nothing.
    pub fn spend(&mut self, path: &DataPath, amount: f64) -> Result<bool, String> {
        if !amount.is_finite() || amount < 0.0 {
            return Err("spend amount must be finite and nonnegative".into());
        }
        let current = self
            .get(path)
            .and_then(Self::number)
            .filter(|n| n.is_finite())
            .ok_or("data field is not a finite number")?;
        if current < amount {
            return Ok(false);
        }
        self.set(path, NbtTag::Double(current - amount))?;
        Ok(true)
    }

    /// Adds to one numeric field with explicit inclusive bounds.
    pub fn add_clamped(
        &mut self,
        path: &DataPath,
        amount: f64,
        min: f64,
        max: f64,
    ) -> Result<f64, String> {
        if ![amount, min, max].into_iter().all(f64::is_finite) || min > max {
            return Err("invalid numeric change or bounds".into());
        }
        let current = self
            .get(path)
            .and_then(Self::number)
            .filter(|n| n.is_finite())
            .ok_or("data field is not a finite number")?;
        let value = (current + amount).clamp(min, max);
        self.set(path, NbtTag::Double(value))?;
        Ok(value)
    }

    /// Revision used by HUD caches and persistence invalidation.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests;
