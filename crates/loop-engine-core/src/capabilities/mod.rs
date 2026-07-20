//! Narrow external-effect contracts consumed by application operations.
//!
//! Capabilities use core facts only. They expose complete commands instead of
//! generic repositories or save/update methods, keeping atomicity explicit.

pub mod audit_export;
pub mod decision_events;
pub mod digest;
pub mod event_attempt_writer;
pub mod id_generator;
pub mod persistence_commands;
pub mod provider_catalog;
pub mod provider_invoker;
pub mod run_reader;
pub mod run_writer;
pub mod time;

use crate::model::bounded::{
    BoundError, BoundedText, COLLECTION_PAGE_DATA_BUDGET_BYTES, COLLECTION_PAGE_MAX_COUNT,
    OPAQUE_INTEGRITY_WIRE_UTF8_BYTES,
};

/// Opaque authenticated continuation token. Integrations own its wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageCursor(BoundedText<OPAQUE_INTEGRITY_WIRE_UTF8_BYTES>);

impl PageCursor {
    pub fn parse(value: impl Into<String>) -> Result<Self, BoundError> {
        Ok(Self(BoundedText::opaque_non_empty("page_cursor", value)?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Count and encoded-byte bounded page request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest<F> {
    limit: u16,
    byte_limit: usize,
    cursor: Option<PageCursor>,
    filter: F,
}

impl<F> PageRequest<F> {
    pub fn new(
        limit: u16,
        byte_limit: usize,
        cursor: Option<PageCursor>,
        filter: F,
    ) -> Result<Self, BoundError> {
        if limit == 0 || usize::from(limit) > usize::from(COLLECTION_PAGE_MAX_COUNT) {
            return Err(BoundError::TooMany {
                field: "page_limit",
                max: usize::from(COLLECTION_PAGE_MAX_COUNT),
                actual: usize::from(limit),
            });
        }
        if byte_limit == 0 || byte_limit > COLLECTION_PAGE_DATA_BUDGET_BYTES {
            return Err(BoundError::EncodedTooLarge {
                field: "page_data",
                max: COLLECTION_PAGE_DATA_BUDGET_BYTES,
                actual: byte_limit,
            });
        }
        Ok(Self {
            limit,
            byte_limit,
            cursor,
            filter,
        })
    }

    pub fn limit(&self) -> u16 {
        self.limit
    }

    pub fn byte_limit(&self) -> usize {
        self.byte_limit
    }

    pub fn cursor(&self) -> Option<&PageCursor> {
        self.cursor.as_ref()
    }

    pub fn filter(&self) -> &F {
        &self.filter
    }
}

/// Stable keyset page. Rows are never truncated to satisfy byte limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub rows: Vec<T>,
    pub next_cursor: Option<PageCursor>,
}

#[cfg(test)]
mod compile_contracts {
    #[test]
    fn narrow_capabilities_are_independently_nameable() {
        fn clock(_: &dyn super::time::TimeSource<Error = ()>) {}
        fn ids(_: &dyn super::id_generator::IdGenerator<Error = ()>) {}
        fn digest(_: &dyn super::digest::DigestComputer<Error = ()>) {}

        let _: fn(&dyn super::time::TimeSource<Error = ()>) = clock;
        let _: fn(&dyn super::id_generator::IdGenerator<Error = ()>) = ids;
        let _: fn(&dyn super::digest::DigestComputer<Error = ()>) = digest;
    }
}
