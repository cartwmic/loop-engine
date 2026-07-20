use crate::capabilities::provider_catalog::{
    CatalogCommandError, CatalogMutation, CatalogMutationResult, ProviderCatalog, ProviderConfig,
    validate_config_revision,
};
use crate::model::ids::RegistrationId;

pub fn execute<C: ProviderCatalog>(
    catalog: &C,
    command: CatalogMutation,
) -> Result<CatalogMutationResult, C::Error> {
    catalog.mutate(command)
}

pub fn result_matches(
    registration_id: &RegistrationId,
    previous_revision: u64,
    result: &CatalogMutationResult,
) -> bool {
    previous_revision.checked_add(1).is_some_and(|next| {
        result.registration.id() == registration_id && result.registration.config_revision() == next
    })
}

pub fn command(
    registration_id: RegistrationId,
    expected_config_revision: u64,
    config: ProviderConfig,
) -> Result<CatalogMutation, CatalogCommandError> {
    validate_config_revision(expected_config_revision)?;
    Ok(CatalogMutation::Update {
        registration_id,
        expected_config_revision,
        config,
    })
}

#[cfg(test)]
mod tests {
    use super::{command, result_matches};
    use crate::capabilities::provider_catalog::{
        CatalogMutation, CatalogMutationResult, ProviderConfig,
    };
    use crate::model::ids::{ProviderHandle, RegistrationId};
    use crate::model::provider::ProviderRegistration;

    #[test]
    fn update_preserves_id_and_requires_positive_revision() {
        let id = RegistrationId::parse("stable").unwrap();
        let config = ProviderConfig::new("/provider", vec![], "/work", 60).unwrap();
        assert!(command(id.clone(), 0, config.clone()).is_err());
        assert!(matches!(
            command(id.clone(), 4, config).unwrap(),
            CatalogMutation::Update {
                registration_id,
                expected_config_revision: 4,
                ..
            } if registration_id == id
        ));
        let result = CatalogMutationResult {
            registration: ProviderRegistration::restore(
                id.clone(),
                Some(ProviderHandle::parse("provider").unwrap()),
                5,
                true,
            )
            .unwrap(),
            affected_active_runs: 0,
            impact_cursor: None,
        };
        assert!(result_matches(&id, 4, &result));
    }
}
