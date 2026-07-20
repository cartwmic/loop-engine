use crate::capabilities::provider_catalog::{
    CatalogCommandError, CatalogMutation, CatalogMutationResult, ProviderCatalog,
    validate_config_revision,
};
use crate::model::ids::{ProviderHandle, RegistrationId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRenameError {
    DuplicateHandle,
}

pub fn execute<C: ProviderCatalog>(
    catalog: &C,
    command: CatalogMutation,
) -> Result<CatalogMutationResult, C::Error> {
    catalog.mutate(command)
}

pub fn map_duplicate_handle(duplicate: bool) -> Result<(), ProviderRenameError> {
    if duplicate {
        Err(ProviderRenameError::DuplicateHandle)
    } else {
        Ok(())
    }
}

pub fn command(
    registration_id: RegistrationId,
    expected_config_revision: u64,
    handle: ProviderHandle,
) -> Result<CatalogMutation, CatalogCommandError> {
    validate_config_revision(expected_config_revision)?;
    Ok(CatalogMutation::Rename {
        registration_id,
        expected_config_revision,
        handle,
    })
}

#[cfg(test)]
mod tests {
    use super::{command, map_duplicate_handle};
    use crate::capabilities::provider_catalog::CatalogMutation;
    use crate::model::ids::{ProviderHandle, RegistrationId};

    #[test]
    fn rename_preserves_registration_identity() {
        let id = RegistrationId::parse("stable").unwrap();
        let handle = ProviderHandle::parse("renamed").unwrap();
        assert!(command(id.clone(), 0, handle.clone()).is_err());
        assert!(map_duplicate_handle(true).is_err());
        assert!(matches!(
            command(id.clone(), 2, handle).unwrap(),
            CatalogMutation::Rename { registration_id, .. } if registration_id == id
        ));
    }
}
