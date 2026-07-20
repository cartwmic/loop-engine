use super::bounded::{BoundError, BoundedText};
use super::ids::{ProviderHandle, RegistrationId};
use super::time::ObservedAt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRegistration {
    id: RegistrationId,
    handle: Option<ProviderHandle>,
    config_revision: u64,
    enabled: bool,
}

impl ProviderRegistration {
    pub fn new(id: RegistrationId, handle: ProviderHandle) -> Self {
        Self {
            id,
            handle: Some(handle),
            config_revision: 1,
            enabled: true,
        }
    }

    pub fn id(&self) -> &RegistrationId {
        &self.id
    }

    pub fn handle(&self) -> Option<&ProviderHandle> {
        self.handle.as_ref()
    }

    pub fn config_revision(&self) -> u64 {
        self.config_revision
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn updated(&self, handle: Option<ProviderHandle>, enabled: bool) -> Option<Self> {
        if enabled != handle.is_some() {
            return None;
        }
        Some(Self {
            id: self.id.clone(),
            handle,
            config_revision: self.config_revision.checked_add(1)?,
            enabled,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigestObservation {
    Observed(BoundedText<256>),
    Unavailable,
}

impl DigestObservation {
    pub fn observed(value: impl Into<String>) -> Result<Self, BoundError> {
        Ok(Self::Observed(BoundedText::opaque_non_empty(
            "provider_digest",
            value,
        )?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderObservation {
    registration_id: RegistrationId,
    locator: BoundedText<4_096>,
    digest: DigestObservation,
    version: Option<BoundedText<256>>,
    observed_at: ObservedAt,
}

impl ProviderObservation {
    pub fn new(
        registration_id: RegistrationId,
        locator: impl Into<String>,
        digest: DigestObservation,
        version: Option<String>,
        observed_at: ObservedAt,
    ) -> Result<Self, BoundError> {
        Ok(Self {
            registration_id,
            locator: BoundedText::opaque_non_empty("provider_locator", locator)?,
            digest,
            version: version
                .map(|value| BoundedText::opaque_non_empty("provider_version", value))
                .transpose()?,
            observed_at,
        })
    }

    pub fn registration_id(&self) -> &RegistrationId {
        &self.registration_id
    }

    pub fn locator(&self) -> &str {
        self.locator.as_str()
    }

    pub fn digest(&self) -> &DigestObservation {
        &self.digest
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_ref().map(|value| value.as_str())
    }

    pub fn observed_at(&self) -> ObservedAt {
        self.observed_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationPhase {
    Started,
    Finished,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::{DigestObservation, ProviderHandle, ProviderRegistration, RegistrationId};

    #[test]
    fn mutable_catalog_facts_do_not_change_registration_identity() {
        let registration = ProviderRegistration::new(
            RegistrationId::parse("stable").unwrap(),
            ProviderHandle::parse("handle").unwrap(),
        );
        let disabled = registration.updated(None, false).unwrap();
        let restored = disabled
            .updated(Some(ProviderHandle::parse("renamed").unwrap()), true)
            .unwrap();
        assert_eq!(restored.id(), registration.id());
        assert_eq!(restored.config_revision(), 3);
        assert_eq!(restored.handle().unwrap().as_str(), "renamed");
        assert!(!disabled.enabled());
        assert!(DigestObservation::observed("sha256:digest").is_ok());
    }
}
