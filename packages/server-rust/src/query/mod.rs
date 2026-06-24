//! Transport-neutral query utilities shared across HTTP sync and the DAG pipeline.

pub mod cursor;
pub mod delta_buffer;
pub mod full_scan_pager;
pub mod sort_key;
pub mod window;
