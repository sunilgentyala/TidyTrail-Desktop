//! Core scanning, layout, and deletion logic for TidyTrail Desktop, kept
//! free of any Tauri/GUI dependency so it can be unit tested on its own and
//! reused unchanged by both the Windows and Linux builds of the app.

mod byte_format;
mod category;
mod scanner;
mod trash;
mod treemap;

pub use byte_format::format_bytes;
pub use category::{categorize, Category};
pub use scanner::{scan, Node, ScanIssue, ScanResult};
pub use trash::{move_all_to_trash, move_to_trash};
pub use treemap::{squarify, Item, Rect};
