use thiserror::Error;

use crate::capabilities::id_generator::IdGenerator;
use crate::capabilities::provider_catalog::{
    CatalogMutation, CatalogMutationResult, ProviderCatalog, ProviderConfig,
};
use crate::model::ids::{ProviderHandle, RegistrationId};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderAddError {
    #[error("provider handle is already enabled")]
    DuplicateHandle,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProviderAddExecutionError<I, C> {
    Id(I),
    Catalog(C),
}

pub fn execute<I: IdGenerator, C: ProviderCatalog>(
    ids: &I,
    catalog: &C,
    handle: ProviderHandle,
    config: ProviderConfig,
) -> Result<CatalogMutationResult, ProviderAddExecutionError<I::Error, C::Error>> {
    let registration_id = ids
        .registration_id()
        .map_err(ProviderAddExecutionError::Id)?;
    catalog
        .mutate(command(registration_id, handle, config))
        .map_err(ProviderAddExecutionError::Catalog)
}

pub fn command(
    registration_id: RegistrationId,
    handle: ProviderHandle,
    config: ProviderConfig,
) -> CatalogMutation {
    CatalogMutation::Add {
        registration_id,
        handle,
        config,
    }
}

pub fn map_duplicate_handle(is_duplicate: bool) -> Result<(), ProviderAddError> {
    if is_duplicate {
        Err(ProviderAddError::DuplicateHandle)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::capabilities::provider_catalog::{CatalogMutation, ProviderConfig};
    use crate::model::ids::{ProviderHandle, RegistrationId};

    use super::{command, map_duplicate_handle};

    #[test]
    fn command_preserves_allocated_identity_and_maps_duplicate() {
        let command = command(
            RegistrationId::parse("stable").unwrap(),
            ProviderHandle::parse("provider").unwrap(),
            ProviderConfig::new("/provider", vec![], "/", 1_000).unwrap(),
        );
        assert!(matches!(command, CatalogMutation::Add { .. }));
        assert!(map_duplicate_handle(true).is_err());
    }
}
