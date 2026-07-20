use loop_engine_core::capabilities::id_generator::IdGenerator;
use loop_engine_core::model::ids::{EvidenceId, JournalId, RegistrationId, RequestId, RunId};
use uuid::Uuid;

#[derive(Debug, Default, Clone, Copy)]
pub struct UuidV7Generator;

impl IdGenerator for UuidV7Generator {
    type Error = std::convert::Infallible;

    fn registration_id(&self) -> Result<RegistrationId, Self::Error> {
        Ok(RegistrationId::parse(Uuid::now_v7().to_string()).expect("UUID is a valid ID"))
    }

    fn run_id(&self) -> Result<RunId, Self::Error> {
        Ok(RunId::parse(Uuid::now_v7().to_string()).expect("UUID is a valid ID"))
    }

    fn request_id(&self) -> Result<RequestId, Self::Error> {
        Ok(RequestId::parse(Uuid::now_v7().to_string()).expect("UUID is a valid ID"))
    }

    fn evidence_id(&self) -> Result<EvidenceId, Self::Error> {
        Ok(EvidenceId::parse(Uuid::now_v7().to_string()).expect("UUID is a valid ID"))
    }

    fn journal_id(&self) -> Result<JournalId, Self::Error> {
        Ok(JournalId::parse(Uuid::now_v7().to_string()).expect("UUID is a valid ID"))
    }
}
