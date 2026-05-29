//! # Generic coroutine driver
//!
//! Every standard-shape coroutine in this crate exposes the same loop
//! contract: report progress through [`M2dirCoroutineState`] and
//! receive replies as its associated [`Arg`]. The [`M2dirCoroutine`]
//! trait unifies that contract behind a single method so a generic
//! driver ([`M2dirClient::run`]) can advance any coroutine without
//! macros.
//!
//! io-m2dir is filesystem-flavoured, so the state enum carries
//! `Wants*` variants for each filesystem (or environmental)
//! primitive the crate emits: directory create / remove / read, file
//! create / read / exists / remove, path rename, and the
//! process id / random bytes inputs needed to mint m2dir entry
//! identifiers. Per-coroutine `Arg` enums only declare the subset of
//! replies that coroutine actually consumes.
//!
//! [`M2dirClient::run`]: crate::client::M2dirClient::run
//! [`Arg`]: M2dirCoroutine::Arg

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use crate::path::M2dirPath;

/// State yielded by a [`M2dirCoroutine`] resume.
///
/// Single generic enum so a generic driver can pattern match on
/// progression without naming a per-coroutine `Result` type.
#[derive(Debug)]
pub enum M2dirCoroutineState<T, E> {
    /// Coroutine terminated successfully with this payload.
    Done(T),

    /// Caller must supply the current process id and feed back the
    /// coroutine's `Arg::Pid` variant.
    WantsPid,

    /// Caller must supply `len` random bytes and feed back the
    /// coroutine's `Arg::Random` variant.
    WantsRandom { len: usize },

    /// Caller must check each path for existence as a regular file
    /// and feed back the coroutine's `Arg::FileExists` variant.
    WantsFileExists(BTreeSet<M2dirPath>),

    /// Caller must list each directory's entries and feed back the
    /// coroutine's `Arg::DirRead` variant.
    WantsDirRead(BTreeSet<M2dirPath>),

    /// Caller must create each directory (with parents) and feed
    /// back the coroutine's `Arg::DirCreate` variant.
    WantsDirCreate(BTreeSet<M2dirPath>),

    /// Caller must recursively remove each directory and feed back
    /// the coroutine's `Arg::DirRemove` variant.
    WantsDirRemove(BTreeSet<M2dirPath>),

    /// Caller must read each file's bytes and feed back the
    /// coroutine's `Arg::FileRead` variant.
    WantsFileRead(BTreeSet<M2dirPath>),

    /// Caller must write each `(path, bytes)` pair and feed back the
    /// coroutine's `Arg::FileCreate` variant.
    WantsFileCreate(BTreeMap<M2dirPath, Vec<u8>>),

    /// Caller must remove each file and feed back the coroutine's
    /// `Arg::FileRemove` variant.
    WantsFileRemove(BTreeSet<M2dirPath>),

    /// Caller must rename each `(from, to)` pair and feed back the
    /// coroutine's `Arg::Rename` variant.
    WantsRename(Vec<(M2dirPath, M2dirPath)>),

    /// Coroutine terminated with this error.
    Err(E),
}

/// Standard-shape m2dir coroutine: anything whose progression maps
/// onto [`M2dirCoroutineState`].
///
/// `resume` is the single source of truth: each implementor's body
/// returns [`M2dirCoroutineState::Done`] / `Wants*` / `Err`
/// directly. [`M2dirClient::run`] drives any [`M2dirCoroutine`] to
/// completion against the local filesystem; downstream code can
/// write its own driver against the same trait.
///
/// [`M2dirClient::run`]: crate::client::M2dirClient::run
pub trait M2dirCoroutine {
    /// Reply fed back into [`resume`](Self::resume) by the driver.
    type Arg;

    /// Payload yielded on terminal success.
    type Output;

    /// Error yielded on terminal failure.
    type Error;

    /// Advances the coroutine one step.
    fn resume(&mut self, arg: Option<Self::Arg>) -> M2dirCoroutineState<Self::Output, Self::Error>;
}
