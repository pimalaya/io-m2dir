//! Collection of I/O-free, resumable, and composable m2dir state
//! machines.
//!
//! Each coroutine emits filesystem requests via the `Wants*` variants
//! of its `*Result` enum (e.g. `WantsDirCreate`, `WantsFileRead`,
//! `WantsFileCreate`, `WantsFileRemove`). The caller performs the
//! matching operation and feeds the corresponding `*Arg` variant back
//! into the next `resume` call to make progress.

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
