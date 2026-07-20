use thiserror::Error;

use crate::capabilities::{Page, PageCursor, PageRequest};
use crate::model::bounded::BoundError;
pub use crate::model::bounded::{
    COLLECTION_PAGE_DATA_BUDGET_BYTES as PAGE_DATA_BUDGET_BYTES,
    COLLECTION_PAGE_DEFAULT_COUNT as DEFAULT_PAGE_COUNT,
    COLLECTION_PAGE_MAX_COUNT as MAX_PAGE_COUNT,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PagingError {
    #[error("page limit must be between 1 and {MAX_PAGE_COUNT}")]
    InvalidLimit,
    #[error("page filter is invalid")]
    InvalidFilter,
    #[error("cursor schema version is unsupported")]
    CursorVersion,
    #[error("cursor operation or filter does not match request")]
    CursorBinding,
    #[error("page truncated without a continuation cursor")]
    MissingContinuation,
    #[error("one row exceeds page byte budget and cannot be truncated")]
    RowTooLarge,
    #[error(transparent)]
    Bound(#[from] BoundError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedCursorBinding {
    pub schema_version: u8,
    pub operation: String,
    pub filter: String,
}

pub fn validate_binding(
    binding: &DecodedCursorBinding,
    operation: &str,
    filter: &str,
) -> Result<(), PagingError> {
    if binding.schema_version != 1 {
        return Err(PagingError::CursorVersion);
    }
    if binding.operation != operation || binding.filter != filter {
        return Err(PagingError::CursorBinding);
    }
    Ok(())
}

pub fn bounded_page<T>(
    rows: impl IntoIterator<Item = (T, usize)>,
    count_limit: usize,
    byte_limit: usize,
    next_cursor: Option<PageCursor>,
) -> Result<Page<T>, PagingError> {
    let mut selected = Vec::new();
    let mut bytes = 0usize;
    let mut truncated = false;
    for (row, size) in rows {
        if size > byte_limit && selected.is_empty() {
            return Err(PagingError::RowTooLarge);
        }
        if selected.len() == count_limit || bytes.saturating_add(size) > byte_limit {
            truncated = true;
            break;
        }
        bytes += size;
        selected.push(row);
    }
    if truncated && next_cursor.is_none() {
        return Err(PagingError::MissingContinuation);
    }
    Ok(Page {
        rows: selected,
        next_cursor,
    })
}

pub fn request<F>(
    limit: Option<u16>,
    cursor: Option<String>,
    filter: F,
) -> Result<PageRequest<F>, PagingError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_COUNT);
    if limit == 0 || limit > MAX_PAGE_COUNT {
        return Err(PagingError::InvalidLimit);
    }
    PageRequest::new(
        limit,
        PAGE_DATA_BUDGET_BYTES,
        cursor.map(PageCursor::parse).transpose()?,
        filter,
    )
    .map_err(PagingError::Bound)
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_PAGE_COUNT, DecodedCursorBinding, MAX_PAGE_COUNT, bounded_page, request,
        validate_binding,
    };

    #[test]
    fn validates_count_and_opaque_cursor_bounds() {
        assert_eq!(request(None, None, ()).unwrap().limit(), DEFAULT_PAGE_COUNT);
        assert!(request(Some(0), None, ()).is_err());
        assert!(request(Some(MAX_PAGE_COUNT + 1), None, ()).is_err());
        assert!(request(Some(1), Some("x".repeat(769)), ()).is_err());
        let wrong_version = DecodedCursorBinding {
            schema_version: 2,
            operation: "run.list".into(),
            filter: "active".into(),
        };
        assert!(validate_binding(&wrong_version, "run.list", "active").is_err());
        let wrong_filter = DecodedCursorBinding {
            schema_version: 1,
            operation: "run.list".into(),
            filter: "all".into(),
        };
        assert!(validate_binding(&wrong_filter, "run.list", "active").is_err());
        assert!(bounded_page([(1, 4), (2, 4)], 10, 4, None).is_err());
        let cursor = crate::capabilities::PageCursor::parse("next").unwrap();
        let page = bounded_page([(1, 4), (2, 4)], 10, 4, Some(cursor)).unwrap();
        assert_eq!(page.rows, vec![1]);
        assert!(bounded_page([(1, 5)], 10, 4, None).is_err());
    }
}
