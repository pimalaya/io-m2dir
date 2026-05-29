//! Collection of I/O-free, resumable, and composable m2dir state
//! machines.
//!
//! Every coroutine reports progression through the unified
//! [`M2dirCoroutineState`](crate::coroutine::M2dirCoroutineState)
//! enum (filesystem-flavoured `Wants*` variants, [`Done`], [`Err`])
//! and consumes its own per-coroutine `Arg` enum on resume. Drive
//! any coroutine end-to-end against the local filesystem via
//! [`M2dirClient::run`](crate::client::M2dirClient::run).
//!
//! [`Done`]: crate::coroutine::M2dirCoroutineState::Done
//! [`Err`]: crate::coroutine::M2dirCoroutineState::Err

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
