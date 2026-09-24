//! Core scanning, layout, and deletion logic for TidyTrail Desktop, kept
//! free of any Tauri/GUI dependency so it can be unit tested on its own and
//! reused unchanged by both the Windows and Linux builds of the app.

mod audit;
mod byte_format;
mod category;
mod delete_guard;
mod scanner;
mod trash;
mod treemap;

pub use audit::{
    trash_with_audit, AuditLog, TrashReport, AUDIT_LOG_DIR_ENV, AUDIT_LOG_FILE_NAME,
    MAX_PATHS_PER_REQUEST,
};
pub use byte_format::format_bytes;
pub use category::{categorize, Category};
pub use delete_guard::{DeleteGuard, DeleteRejection};
pub use scanner::{is_virtual_filesystem, scan, Node, ScanIssue, ScanResult, MAX_DEPTH};
pub use trash::{move_all_to_trash, move_to_trash};
pub use treemap::{squarify, Item, Rect};
