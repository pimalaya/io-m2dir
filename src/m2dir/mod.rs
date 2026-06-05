//! M2dir-level coroutines: create, delete, list.

pub mod create;
pub mod delete;
pub mod list;
#[cfg(not(feature = "client"))]
mod parse;
pub mod types;
pub mod utils;
