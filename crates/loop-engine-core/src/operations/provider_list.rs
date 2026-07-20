use crate::capabilities::provider_catalog::{
    ActiveRunImpact, ProviderCatalog, ProviderCatalogRow, ProviderListFilter,
};
use crate::capabilities::{Page, PageRequest};
use crate::model::ids::RegistrationId;
use crate::operations::paging::{PagingError, request};

pub fn execute_list<C: ProviderCatalog>(
    catalog: &C,
    request: &PageRequest<ProviderListFilter>,
) -> Result<Page<ProviderCatalogRow>, C::Error> {
    catalog.list(request)
}

pub fn execute_impact<C: ProviderCatalog>(
    catalog: &C,
    registration_id: &RegistrationId,
    request: &PageRequest<()>,
) -> Result<Page<ActiveRunImpact>, C::Error> {
    catalog.active_run_impact(registration_id, request)
}

pub fn parse_filter(value: &str) -> Result<ProviderListFilter, PagingError> {
    match value {
        "enabled" => Ok(ProviderListFilter::Enabled),
        "tombstoned" => Ok(ProviderListFilter::Tombstoned),
        "all" => Ok(ProviderListFilter::All),
        _ => Err(PagingError::InvalidFilter),
    }
}

pub fn list_request(
    filter: ProviderListFilter,
    limit: Option<u16>,
    cursor: Option<String>,
) -> Result<PageRequest<ProviderListFilter>, PagingError> {
    request(limit, cursor, filter)
}

pub fn impact_request(
    limit: Option<u16>,
    cursor: Option<String>,
) -> Result<PageRequest<()>, PagingError> {
    request(limit, cursor, ())
}

#[cfg(test)]
mod tests {
    use super::{impact_request, list_request, parse_filter};

    #[test]
    fn filters_and_both_page_shapes_are_validated() {
        assert!(parse_filter("enabled").is_ok());
        assert!(parse_filter("unknown").is_err());
        assert!(list_request(parse_filter("all").unwrap(), Some(200), None).is_ok());
        assert!(impact_request(Some(1_001), None).is_err());
    }
}
