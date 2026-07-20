use crate::capabilities::run_reader::{EvidenceInventoryRow, RunReader};
use crate::capabilities::{Page, PageRequest};
use crate::model::ids::RunId;
use crate::operations::paging::{PagingError, request};

pub fn execute<R: RunReader>(
    reader: &R,
    run_id: &RunId,
    request: &PageRequest<()>,
) -> Result<Page<EvidenceInventoryRow>, R::Error> {
    reader.evidence(run_id, request)
}

pub fn query(limit: Option<u16>, cursor: Option<String>) -> Result<PageRequest<()>, PagingError> {
    request(limit, cursor, ())
}

#[cfg(test)]
mod tests {
    #[test]
    fn evidence_query_rejects_oversized_cursor() {
        assert!(super::query(None, None).is_ok());
        assert!(super::query(None, Some("x".repeat(769))).is_err());
    }
}
