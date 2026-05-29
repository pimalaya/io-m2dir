//! Collection of I/O-free, resumable, and composable m2dir state
//! machines.
//!
//! Every coroutine implements
//! [`M2dirCoroutine`](crate::coroutine::M2dirCoroutine) with `Yield =
//! M2dirYield` and `Return = Result<Output, Error>`, then advances
//! one step at a time through
//! [`M2dirCoroutineState`](crate::coroutine::M2dirCoroutineState)
//! (`Yielded` for filesystem and environmental requests, `Complete`
//! for terminal output). Drive any coroutine end-to-end against the
//! local filesystem via
//! [`M2dirClient::run`](crate::client::M2dirClient::run).

pub mod flag_add;
pub mod flag_remove;
pub mod flag_set;
pub mod mailbox_create;
pub mod mailbox_delete;
pub mod mailbox_list;
pub mod message_delete;
pub mod message_get;
pub mod message_list;
pub mod message_store;
