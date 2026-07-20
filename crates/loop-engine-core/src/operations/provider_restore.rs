use crate::capabilities::provider_catalog::{
    CatalogCommandError, CatalogMutation, CatalogMutationResult, ProviderCatalog, ProviderConfig,
    validate_config_revision,
};
use crate::model::ids::{ProviderHandle, RegistrationId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRestoreError {
    HandleOccupied,
}

pub fn execute<C: ProviderCatalog>(
    catalog: &C,
    command: CatalogMutation,
) -> Result<CatalogMutationResult, C::Error> {
    catalog.mutate(command)
}

pub fn map_occupied_handle(occupied: bool) -> Result<(), ProviderRestoreError> {
    if occupied {
        Err(ProviderRestoreError::HandleOccupied)
    } else {
        Ok(())
    }
}

pub fn command(
    registration_id: RegistrationId,
    expected_config_revision: u64,
    handle: ProviderHandle,
    config: ProviderConfig,
) -> Result<CatalogMutation, CatalogCommandError> {
    validate_config_revision(expected_config_revision)?;
    Ok(CatalogMutation::Restore {
        registration_id,
        expected_config_revision,
        handle,
        config,
    })
}

#[cfg(test)]
mod tests {
    use super::{command, map_occupied_handle};
    use crate::capabilities::provider_catalog::{CatalogMutation, ProviderConfig};
    use crate::model::ids::{ProviderHandle, RegistrationId};

    #[test]
    fn restore_targets_exact_tombstoned_identity() {
        let id = RegistrationId::parse("tombstone").unwrap();
        let handle = ProviderHandle::parse("restored").unwrap();
        let config = ProviderConfig::new("/provider", vec![], "/work", 60).unwrap();
        assert!(command(id.clone(), 0, handle.clone(), config.clone()).is_err());
        assert!(map_occupied_handle(true).is_err());
        assert!(matches!(
            command(id.clone(), 3, handle, config).unwrap(),
            CatalogMutation::Restore { registration_id, .. } if registration_id == id
        ));
    }
}
