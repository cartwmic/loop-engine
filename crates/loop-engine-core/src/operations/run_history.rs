use crate::capabilities::run_reader::RunHistoryReader;
use crate::capabilities::{Page, PageRequest};
use crate::model::ids::RunId;
use crate::model::journal::JournalEntry;
use crate::operations::paging::{PagingError, request};

pub fn execute<R: RunHistoryReader>(
    reader: &R,
    run_id: &RunId,
    request: &PageRequest<()>,
) -> Result<Page<JournalEntry>, R::Error> {
    reader.history(run_id, request)
}

pub fn query(limit: Option<u16>, cursor: Option<String>) -> Result<PageRequest<()>, PagingError> {
    request(limit, cursor, ())
}

#[cfg(test)]
mod tests {
    #[test]
    fn history_query_enforces_page_bounds() {
        assert!(super::query(Some(1), None).is_ok());
        assert!(super::query(Some(0), None).is_err());
        assert!(super::query(Some(1_001), None).is_err());
    }
}
