//! Typed custom data shared by players and other entities through namespaced paths.
//! Paths look like `magic:state/mana/current`. Reads are owned snapshots; writes
//! merge one field and preserve other extensions' data. Numeric operations are atomic.

pub use crate::block_behavior::types::{DataValue as Value, EntityData as Store};

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "Borrowed ABI handles follow the shared receiver convention of the Server API"
)]
#[stabby::host_api]
/// Typed values and one checked custom-data store for every entity.
pub mod api {
    use super::{Store, Value};
    use crate::block_behavior::types::Entity;
    #[cfg(feature = "host")]
    use crate::host::data::Target;
    #[cfg(feature = "host")]
    use crate::players::Uuid;
    use crate::{ApiResult, PlayerId, Server, World, entities::EntityId};
    #[cfg(feature = "host")]
    use simdnbt::owned::{NbtCompound, NbtList, NbtTag};
    use stabby::{
        option::Option as ApiOption, str::Str, string::String as ApiString, vec::Vec as ApiVec,
    };
    #[cfg(feature = "host")]
    use steel_core::entity::custom_data::{CustomData, DataPath};

    impl Value {
        /// Creates an empty compound.
        #[must_use]
        pub fn compound() -> Self {
            Self::from_host(NbtTag::Compound(NbtCompound::new()))
        }
        /// Creates a signed integer.
        #[must_use]
        pub fn integer(value: i64) -> Self {
            Self::from_host(NbtTag::Long(value))
        }
        /// Creates a finite floating-point number.
        #[must_use]
        pub fn number(value: f64) -> ApiResult<Self> {
            if !value.is_finite() {
                return Err(ApiString::from("data numbers must be finite")).into();
            }
            Ok(Self::from_host(NbtTag::Double(value))).into()
        }
        /// Creates text.
        #[must_use]
        pub fn text(value: Str<'_>) -> Self {
            Self::from_host(NbtTag::String(value.as_str().into()))
        }
        /// Creates a boolean, stored as an NBT byte.
        #[must_use]
        pub fn boolean(value: bool) -> Self {
            Self::from_host(NbtTag::Byte(i8::from(value)))
        }
        /// Creates an opaque byte array, suitable for serialized native entity state.
        #[must_use]
        pub fn bytes(value: ApiVec<u8>) -> Self {
            Self::from_host(NbtTag::ByteArray(value.into_iter().collect()))
        }
        /// Creates a homogeneous list of compound records.
        #[must_use]
        pub fn records(values: ApiVec<Self>) -> ApiResult<Self> {
            let result = (|| -> Result<Self, ApiString> {
                let mut records = Vec::with_capacity(values.len());
                for value in values {
                    let NbtTag::Compound(value) = value.into_host() else {
                        return Err("records require compound values".into());
                    };
                    records.push(value);
                }
                Ok(Self::from_host(NbtTag::List(NbtList::Compound(records))))
            })();
            result.into()
        }
        /// Sets one compound field, replacing only that field.
        pub fn put(&mut self, key: Str<'_>, value: Self) -> ApiResult<()> {
            let Some(compound) = self.host_mut().compound_mut() else {
                return Err(ApiString::from("value is not a compound")).into();
            };
            if let Some(previous) = compound.get_mut(key.as_str()) {
                *previous = value.into_host();
            } else {
                compound.insert(key.as_str(), value.into_host());
            }
            Ok(()).into()
        }
        /// Reads one compound field as an independent snapshot.
        #[must_use]
        pub fn get(&self, key: Str<'_>) -> ApiOption<Self> {
            self.host()
                .compound()
                .and_then(|c| c.get(key.as_str()))
                .cloned()
                .map(Self::from_host)
                .into()
        }
        /// Reads a compound record by index.
        #[must_use]
        pub fn record(&self, index: u32) -> ApiOption<Self> {
            self.host()
                .list()
                .and_then(NbtList::compounds)
                .and_then(|list| list.get(index as usize))
                .cloned()
                .map(|c| Self::from_host(NbtTag::Compound(c)))
                .into()
        }
        /// Number of records in a compound list; errors for other kinds.
        pub fn record_count(&self) -> ApiResult<u32> {
            self.host()
                .list()
                .and_then(NbtList::compounds)
                .ok_or_else(|| ApiString::from("value is not a record list"))
                .and_then(|list| {
                    u32::try_from(list.len()).map_err(|e| ApiString::from(e.to_string()))
                })
                .into()
        }
        /// Reads a numeric value, promoting NBT numeric types to double.
        #[must_use]
        pub fn as_number(&self) -> ApiOption<f64> {
            CustomData::number(self.host()).into()
        }
        /// Reads an exact signed integer.
        #[must_use]
        pub fn as_integer(&self) -> ApiOption<i64> {
            self.host().long().into()
        }
        /// Reads text.
        #[must_use]
        pub fn as_text(&self) -> ApiOption<ApiString> {
            self.host()
                .string()
                .map(|s| ApiString::from(s.to_string()))
                .into()
        }
        /// Reads a byte array.
        #[must_use]
        pub fn as_bytes(&self) -> ApiOption<ApiVec<u8>> {
            self.host()
                .byte_array()
                .map(|v| v.iter().copied().collect())
                .into()
        }
    }

    impl<'a> Server<'a> {
        /// Opens this player incarnation's current-domain custom data.
        /// Follows its active gameplay domain; reacquire after respawn.
        pub fn player_data(&self, player: PlayerId) -> ApiResult<Store> {
            Target::player(self.host(), player)
                .map(Store::from_host)
                .map_err(ApiString::from)
                .into()
        }
    }
    impl<'a> World<'a> {
        /// Opens custom data for a loaded entity, including a player.
        /// Its network ID, UUID, and runtime generation are checked on every operation.
        pub fn entity_data(&self, entity: EntityId) -> ApiResult<Store> {
            Target::entity(self.host(), entity)
                .map(Store::from_host)
                .map_err(ApiString::from)
                .into()
        }
    }
    impl<'a> Entity<'a> {
        /// Opens the same store from a block, fluid, or custom-mob callback.
        pub fn data(&self) -> ApiResult<Store> {
            let result = (|| -> Result<_, String> {
                let entity = *self.host();
                let world = entity.level().ok_or("data target has no world")?;
                Target::entity(
                    &world,
                    EntityId {
                        id: entity.id(),
                        uuid: Uuid {
                            bytes: *entity.uuid().as_bytes(),
                        },
                    },
                )
                .map(Store::from_host)
            })();
            result.map_err(ApiString::from).into()
        }
    }
    impl Store {
        /// Reads an owned typed snapshot; a missing field returns None.
        pub fn get(&self, path: Str<'_>) -> ApiResult<ApiOption<Value>> {
            self.host()
                .get(path.as_str())
                .map(|value| value.map(Value::from_host).into())
                .map_err(ApiString::from)
                .into()
        }
        /// Replaces one namespaced field, preserving siblings and native data.
        pub fn set(&self, path: Str<'_>, value: Value) -> ApiResult<()> {
            let result = (|| -> Result<_, String> {
                let path = DataPath::parse(path.as_str())?;
                self.host()
                    .update(|data| data.set(&path, value.into_host()))
            })();
            result.map_err(ApiString::from).into()
        }
        /// Initializes a missing field atomically, preserving an existing value.
        pub fn initialize(&self, path: Str<'_>, value: Value) -> ApiResult<bool> {
            let result = (|| -> Result<_, String> {
                let path = DataPath::parse(path.as_str())?;
                self.host()
                    .update(|data| data.initialize(&path, value.into_host()))
            })();
            result.map_err(ApiString::from).into()
        }
        /// Atomically spends a nonnegative amount; insufficient funds change nothing.
        pub fn spend(&self, path: Str<'_>, amount: f64) -> ApiResult<bool> {
            let result = (|| -> Result<_, String> {
                let path = DataPath::parse(path.as_str())?;
                self.host().update(|data| data.spend(&path, amount))
            })();
            result.map_err(ApiString::from).into()
        }
        /// Atomically adds a number with inclusive bounds.
        pub fn add_clamped(
            &self,
            path: Str<'_>,
            amount: f64,
            min: f64,
            max: f64,
        ) -> ApiResult<f64> {
            let result = (|| -> Result<_, String> {
                let path = DataPath::parse(path.as_str())?;
                self.host()
                    .update(|data| data.add_clamped(&path, amount, min, max))
            })();
            result.map_err(ApiString::from).into()
        }
        /// Removes only the selected field; reports whether it existed.
        pub fn remove(&self, path: Str<'_>) -> ApiResult<bool> {
            let result = (|| -> Result<_, String> {
                let path = DataPath::parse(path.as_str())?;
                self.host().update(|data| Ok(data.remove(&path).is_some()))
            })();
            result.map_err(ApiString::from).into()
        }
    }
}
