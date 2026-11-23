#[stabby::host_api]
/// Content registration capabilities borrowed during startup.
pub mod api {
    #[cfg(feature = "host")]
    use crate::host::content::{self, ContentRegistration};
    use crate::{
        ApiResult, BlockState, Registry,
        block_behavior::{DynBlockBehavior, OriginalBlockBehavior},
        content::{
            BedDefinition, BiomeDefinition, BlockDefinition, BlockItemDefinition, EntityDefinition,
            FluidDefinition, ItemDefinition, RegisteredFluid, SlabMaterial, TextureFallback,
        },
    };
    use stabby::{slice::Slice, str::Str, string::String as ApiString, vec::Vec as ApiVec};

    /// Exclusive access before the real item and block registries freeze.
    #[opaque]
    pub struct ContentRegistry<'a>(&'a mut ContentRegistration<'a>);

    impl<'a> ContentRegistry<'a> {
        /// Discovers every registered vanilla slab and its placing item.
        #[must_use]
        pub fn slab_materials(&self) -> ApiVec<SlabMaterial> {
            content::slab_materials(self.host().registry)
        }

        /// Registers a complete bed state family and its placing item.
        pub fn register_bed(&mut self, definition: BedDefinition) -> ApiResult<BlockState> {
            content::register_bed(self.host_mut().registry, definition)
                .map_err(ApiString::from)
                .into()
        }

        /// This extension's TOML configuration.
        #[must_use]
        pub fn config(&self) -> ApiString {
            self.host().config.into()
        }

        /// Adds a namespaced asset under `assets/` to the combined server pack.
        pub fn add_resource_asset(&mut self, path: Str<'_>, bytes: Slice<'_, u8>) -> ApiResult<()> {
            content::resource_pack::add_asset(
                self.host_mut().registry,
                path.as_str(),
                bytes.as_slice(),
            )
            .map_err(ApiString::from)
            .into()
        }

        /// Adds a PNG and creates a flat item model with the supplied namespaced key.
        pub fn register_item_texture(
            &mut self,
            model: Str<'_>,
            png: Slice<'_, u8>,
        ) -> ApiResult<()> {
            content::resource_pack::register_texture(
                self.host_mut().registry,
                model.as_str(),
                png.as_slice(),
                false,
            )
            .map_err(ApiString::from)
            .into()
        }

        /// Adds a PNG and creates a cube model for a placed block or its item.
        pub fn register_block_texture(
            &mut self,
            model: Str<'_>,
            png: Slice<'_, u8>,
        ) -> ApiResult<()> {
            content::resource_pack::register_texture(
                self.host_mut().registry,
                model.as_str(),
                png.as_slice(),
                true,
            )
            .map_err(ApiString::from)
            .into()
        }

        /// Registers a block, its placing item, and a shared cube model from one PNG.
        /// Uses the block's key for the model and applies the fallback per client.
        pub fn register_textured_cube(
            &mut self,
            definition: BlockItemDefinition,
            png: Slice<'_, u8>,
            fallback: TextureFallback,
        ) -> ApiResult<BlockState> {
            self.host_mut()
                .register_block(None, |registry| {
                    content::resource_pack::register_textured_cube(
                        registry,
                        definition,
                        png.as_slice(),
                        fallback,
                    )
                })
                .map_err(ApiString::from)
                .into()
        }

        /// Registers a textured cube with its placing item and custom behavior.
        /// The factory runs after content freezes and receives the registry, the
        /// block's default state, and its default behavior for delegation.
        pub fn register_textured_cube_with_behavior(
            &mut self,
            definition: BlockItemDefinition,
            png: Slice<'_, u8>,
            fallback: TextureFallback,
            factory: extern "C" fn(
                Registry<'_>,
                BlockState,
                OriginalBlockBehavior,
            ) -> ApiResult<DynBlockBehavior>,
        ) -> ApiResult<BlockState> {
            self.host_mut()
                .register_block(Some(factory), |registry| {
                    content::resource_pack::register_textured_cube(
                        registry,
                        definition,
                        png.as_slice(),
                        fallback,
                    )
                })
                .map_err(ApiString::from)
                .into()
        }

        /// Uses a Minecraft head as the no-pack fallback for a contributed item model.
        /// Pass a built-in cubical head key or a Minecraft texture URL or hash.
        /// Use only with centered cube geometry when the model is used in block displays.
        pub fn register_head_fallback(&mut self, model: Str<'_>, skin: Str<'_>) -> ApiResult<()> {
            content::resource_pack::register_head_fallback(
                self.host_mut().registry,
                model.as_str(),
                skin.as_str(),
            )
            .map_err(ApiString::from)
            .into()
        }

        /// Registers a non-placing item with default item behavior.
        pub fn register_item(&mut self, definition: ItemDefinition) -> ApiResult<()> {
            content::register_item(self.host_mut().registry, definition)
                .map_err(ApiString::from)
                .into()
        }

        /// Registers a block without a placing item or default loot table.
        pub fn register_block(&mut self, definition: BlockDefinition) -> ApiResult<BlockState> {
            self.host_mut()
                .register_block(None, |registry| {
                    content::register_block(registry, definition)
                })
                .map_err(ApiString::from)
                .into()
        }

        /// Registers a block and its custom behavior without a placing item.
        /// The factory runs after content freezes, before behavior replacement hooks.
        pub fn register_block_with_behavior(
            &mut self,
            definition: BlockDefinition,
            factory: extern "C" fn(
                Registry<'_>,
                BlockState,
                OriginalBlockBehavior,
            ) -> ApiResult<DynBlockBehavior>,
        ) -> ApiResult<BlockState> {
            self.host_mut()
                .register_block(Some(factory), |registry| {
                    content::register_block(registry, definition)
                })
                .map_err(ApiString::from)
                .into()
        }

        /// Registers a mob type that clients see as its vanilla carrier.
        pub fn register_entity(&mut self, definition: EntityDefinition) -> ApiResult<()> {
            content::register_entity(self.host_mut().registry, definition)
                .map_err(ApiString::from)
                .into()
        }

        /// Registers a biome that clients receive and chunks save by key.
        pub fn register_biome(&mut self, definition: BiomeDefinition) -> ApiResult<()> {
            content::register_biome(self.host_mut().registry, definition)
                .map_err(ApiString::from)
                .into()
        }

        /// Registers a fluid pair, its 16-state liquid block, and its optional bucket.
        pub fn register_fluid(
            &mut self,
            definition: FluidDefinition,
        ) -> ApiResult<RegisteredFluid> {
            content::register_fluid(self.host_mut().registry, definition)
                .map_err(ApiString::from)
                .into()
        }

        /// Registers a block, its placing item, and optionally its self-drop.
        pub fn register_block_item(
            &mut self,
            definition: BlockItemDefinition,
        ) -> ApiResult<BlockState> {
            self.host_mut()
                .register_block(None, |registry| {
                    content::register_block_item(registry, definition)
                })
                .map_err(ApiString::from)
                .into()
        }

        /// Registers a block, its placing item, optional self-drop, and custom behavior.
        /// The factory receives the block's default state and behavior, including placement
        /// and self-drop, and runs before behavior replacement hooks.
        pub fn register_block_item_with_behavior(
            &mut self,
            definition: BlockItemDefinition,
            factory: extern "C" fn(
                Registry<'_>,
                BlockState,
                OriginalBlockBehavior,
            ) -> ApiResult<DynBlockBehavior>,
        ) -> ApiResult<BlockState> {
            self.host_mut()
                .register_block(Some(factory), |registry| {
                    content::register_block_item(registry, definition)
                })
                .map_err(ApiString::from)
                .into()
        }
    }

    impl<'a> crate::Hooks<'a> {
        /// Subscribes during pre-init. Content is registered before behaviors,
        /// commands, extension init, and world loading.
        pub fn register_content_registration(
            &mut self,
            callback: extern "C" fn(ContentRegistry<'_>) -> ApiResult<()>,
        ) -> ApiResult<()> {
            content::subscribe(self.host_mut(), callback)
                .map_err(ApiString::from)
                .into()
        }
    }
}
