//! Progress rendering utilities for TLC runs.

pub mod ndjson;
pub mod tty;

pub use ndjson::{NdjsonWriter, ProgressEvent};
pub use tty::{TtyOptions, TtyRenderer};
