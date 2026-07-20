use crate::capabilities::PageCursor;
use crate::capabilities::provider_catalog::{
    ActiveSetSnapshot, CatalogMutation, CatalogMutationResult, DisableAcknowledgement,
    ProviderCatalog,
};
use crate::model::ids::RegistrationId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisableWarningPage {
    pub snapshot: ActiveSetSnapshot,
    pub next_cursor: Option<PageCursor>,
    pub acknowledgement: Option<DisableAcknowledgement>,
}

impl DisableWarningPage {
    pub fn can_authorize(&self) -> bool {
        self.next_cursor.is_none() && self.acknowledgement.is_some()
    }
}

pub fn execute<C: ProviderCatalog>(
    catalog: &C,
    command: CatalogMutation,
) -> Result<CatalogMutationResult, C::Error> {
    catalog.mutate(command)
}

pub fn command(
    registration_id: RegistrationId,
    page: DisableWarningPage,
) -> Option<CatalogMutation> {
    if !page.can_authorize() {
        return None;
    }
    Some(CatalogMutation::Disable {
        registration_id,
        expected: page.snapshot,
        acknowledgement: page.acknowledgement.expect("checked above"),
    })
}

#[cfg(test)]
mod tests {
    use crate::capabilities::PageCursor;
    use crate::capabilities::provider_catalog::{ActiveSetSnapshot, DisableAcknowledgement};
    use crate::model::ids::RegistrationId;

    use super::{DisableWarningPage, command};

    fn snapshot() -> ActiveSetSnapshot {
        ActiveSetSnapshot::new(2, "sha256:x", 1).unwrap()
    }

    #[test]
    fn only_final_warning_page_authorizes() {
        let intermediate = DisableWarningPage {
            snapshot: snapshot(),
            next_cursor: Some(PageCursor::parse("next").unwrap()),
            acknowledgement: None,
        };
        assert!(command(RegistrationId::parse("r").unwrap(), intermediate).is_none());
        let final_page = DisableWarningPage {
            snapshot: snapshot(),
            next_cursor: None,
            acknowledgement: Some(DisableAcknowledgement::parse("ack").unwrap()),
        };
        assert!(command(RegistrationId::parse("r").unwrap(), final_page).is_some());
    }
}
