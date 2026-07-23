use crate::capabilities::run_reader::{RunCatalogReader, RunListFilter, RunListRow};
use crate::capabilities::{Page, PageRequest};
use crate::operations::paging::{PagingError, request};

pub fn execute<R: RunCatalogReader>(
    reader: &R,
    request: &PageRequest<RunListFilter>,
) -> Result<Page<RunListRow>, R::Error> {
    reader.list(request)
}

pub fn parse_filter(value: &str) -> Result<RunListFilter, PagingError> {
    match value {
        "active" => Ok(RunListFilter::Active),
        "terminal" => Ok(RunListFilter::Terminal),
        "all" => Ok(RunListFilter::All),
        _ => Err(PagingError::InvalidFilter),
    }
}

pub fn query(
    filter: RunListFilter,
    limit: Option<u16>,
    cursor: Option<String>,
) -> Result<PageRequest<RunListFilter>, PagingError> {
    request(limit, cursor, filter)
}

#[cfg(test)]
mod tests {
    use super::{parse_filter, query};

    #[test]
    fn active_is_explicit_default_shape_and_invalid_filters_fail() {
        let active = parse_filter("active").unwrap();
        assert!(query(active, None, None).is_ok());
        assert!(parse_filter("label").is_err());
    }
}
