mod error;
mod event;
mod rotation;
mod schema;
mod writer;

pub use error::TraceError;
pub use event::{TRACE_SCHEMA_VERSION, TraceCategory, TraceEvent};
pub use rotation::{
    TRACE_DIRECTORY_BUDGET_BYTES, TRACE_FILE_MAX_BYTES, TRACE_INIT_RESERVATION_BYTES,
    TRACE_PROVIDER_CALL_RESERVATION_BYTES, TRACE_RETAINED_FILES_MAX,
};
pub use schema::trace_event_schema;
pub use writer::TraceWriter;
