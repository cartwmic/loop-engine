use crate::capabilities::audit_export::{AuditExporter, AuditSnapshot, ExportTarget};
use crate::model::ids::RunId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    pub run_id: RunId,
    pub target: ExportTarget,
}

pub fn execute<E: AuditExporter>(
    exporter: &E,
    request: &ExportRequest,
) -> Result<AuditSnapshot, E::Error> {
    exporter.export_consistent(&request.run_id, &request.target)
}

pub fn request(run_id: RunId, target: ExportTarget) -> ExportRequest {
    ExportRequest { run_id, target }
}

#[cfg(test)]
mod tests {
    use crate::capabilities::audit_export::ExportTarget;
    use crate::model::ids::RunId;

    #[test]
    fn export_request_preserves_exact_target() {
        let request = super::request(
            RunId::parse("run-1").unwrap(),
            ExportTarget::parse("/tmp/export").unwrap(),
        );
        assert_eq!(request.target.as_str(), "/tmp/export");
    }
}
