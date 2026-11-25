use crate::{ApiResult, ApiResultExt, Hooks, IntoApiResult, callbacks::Table, stabby};

use super::ContentRegistry;

type Registrar = dyn Fn(&mut ContentRegistry<'_>) -> Result<(), String> + Send + Sync;
static REGISTRARS: Table<Registrar> = Table::new();

#[stabby::export(callback)]
fn register(mut registry: ContentRegistry<'_>, token: u64) -> ApiResult<()> {
    let result = REGISTRARS
        .get(token)
        .ok_or_else(|| format!("missing content registration callback {token}"))
        .and_then(|registrar| registrar(&mut registry));
    result.into_api_result()
}

impl Hooks<'_> {
    /// Runs a Rust callback with the mutable content registry before it freezes.
    /// Register during pre-init. Each callback runs at its original registration position.
    pub fn on_content(
        &mut self,
        registrar: impl Fn(&mut ContentRegistry<'_>) -> Result<(), String> + Send + Sync + 'static,
    ) -> Result<(), String> {
        let token = REGISTRARS.insert(Box::new(registrar));
        self.register_content_callback(register, token)
            .into_result()
    }
}
