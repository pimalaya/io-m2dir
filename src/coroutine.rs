//! # Generator-shape coroutine driver
//!
//! Mirrors the shape of `core::ops::Coroutine`: a `Yield` associated
//! type for intermediate progress, a `Return` associated type for
//! terminal output, and a two-variant [`M2dirCoroutineState`]
//! (`Yielded` / `Complete`).
//!
//! io-m2dir is filesystem-flavoured, so every coroutine in the crate
//! picks the standard [`M2dirYield`] directly: it gathers every
//! filesystem (and environmental) primitive the crate emits, namely
//! directory create / remove / read, file create / read / exists /
//! remove, path rename, and the process id / random bytes inputs
//! needed to mint m2dir entry identifiers.
//!
//! [`M2dirClient::run`] drives any standard-Yield coroutine to
//! completion against the local filesystem.
//!
//! [`M2dirClient::run`]: crate::client::M2dirClient::run

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use crate::path::M2dirPath;

/// State yielded by an [`M2dirCoroutine::resume`] step.
///
/// Two-variant by design (matches std's `core::ops::CoroutineState`):
/// any further variation lives inside the per-coroutine `Yield` type.
#[derive(Debug)]
pub enum M2dirCoroutineState<Y, R> {
    /// Intermediate yield. The driver reacts to `Y` (do filesystem
    /// I/O, supply pid / random bytes…) and resumes the coroutine
    /// again.
    Yielded(Y),
    /// Terminal yield. By convention `R = Result<Output, Error>`.
    Complete(R),
}

/// Standard-shape m2dir coroutine.
///
/// Implementors own their internal state machine and declare their
/// per-step `Yield` plus a terminal `Return`. The driver reacts to
/// each `Yield` variant and resumes until `Complete`.
pub trait M2dirCoroutine {
    /// Intermediate value handed back on every step. Per-coroutine:
    /// each implementor picks exactly the variants it needs. In
    /// io-m2dir every coroutine picks [`M2dirYield`].
    type Yield;
    /// Terminal value. By convention `Result<Output, Error>`; the "ok"
    /// arm carries the operation's final output, the "error" arm
    /// carries the cause.
    type Return;

    /// Advances the coroutine one step.
    ///
    /// Pass [`None`] on the initial call. Pass `Some(arg)` carrying
    /// the value matching the previous `Yielded` variant.
    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return>;
}

/// Standard filesystem-only Yield. Every io-m2dir coroutine picks
/// `type Yield = M2dirYield`.
///
/// Each variant is paired with the matching [`M2dirArg`] variant the
/// driver feeds back on the next `resume`.
#[derive(Debug)]
pub enum M2dirYield {
    /// Driver must supply the current process id and feed back
    /// [`M2dirArg::Pid`].
    WantsPid,

    /// Driver must supply `len` random bytes and feed back
    /// [`M2dirArg::Random`].
    WantsRandom { len: usize },

    /// Driver must check each path for existence as a regular file
    /// and feed back [`M2dirArg::FileExists`].
    WantsFileExists(BTreeSet<M2dirPath>),

    /// Driver must list each directory's entries and feed back
    /// [`M2dirArg::DirRead`].
    WantsDirRead(BTreeSet<M2dirPath>),

    /// Driver must create each directory (with parents) and feed
    /// back [`M2dirArg::DirCreate`].
    WantsDirCreate(BTreeSet<M2dirPath>),

    /// Driver must recursively remove each directory and feed back
    /// [`M2dirArg::DirRemove`].
    WantsDirRemove(BTreeSet<M2dirPath>),

    /// Driver must read each file's bytes and feed back
    /// [`M2dirArg::FileRead`].
    WantsFileRead(BTreeSet<M2dirPath>),

    /// Driver must write each `(path, bytes)` pair and feed back
    /// [`M2dirArg::FileCreate`].
    WantsFileCreate(BTreeMap<M2dirPath, Vec<u8>>),

    /// Driver must remove each file and feed back
    /// [`M2dirArg::FileRemove`].
    WantsFileRemove(BTreeSet<M2dirPath>),

    /// Driver must rename each `(from, to)` pair and feed back
    /// [`M2dirArg::Rename`].
    WantsRename(Vec<(M2dirPath, M2dirPath)>),
}

/// Reply fed back into [`M2dirCoroutine::resume`] by the driver.
///
/// Each variant matches the corresponding [`M2dirYield`] request and
/// carries the value the driver gathered while servicing it.
#[derive(Clone, Debug)]
pub enum M2dirArg {
    /// Reply to [`M2dirYield::WantsPid`].
    Pid(u32),
    /// Reply to [`M2dirYield::WantsRandom`].
    Random(Vec<u8>),
    /// Reply to [`M2dirYield::WantsFileExists`]: probed path to
    /// whether it exists as a regular file.
    FileExists(BTreeMap<M2dirPath, bool>),
    /// Reply to [`M2dirYield::WantsDirRead`]: probed directory to
    /// the set of paths found inside.
    DirRead(BTreeMap<M2dirPath, BTreeSet<M2dirPath>>),
    /// Reply to [`M2dirYield::WantsDirCreate`].
    DirCreate,
    /// Reply to [`M2dirYield::WantsDirRemove`].
    DirRemove,
    /// Reply to [`M2dirYield::WantsFileRead`]: probed file to its
    /// bytes (or an empty buffer when the file is missing).
    FileRead(BTreeMap<M2dirPath, Vec<u8>>),
    /// Reply to [`M2dirYield::WantsFileCreate`].
    FileCreate,
    /// Reply to [`M2dirYield::WantsFileRemove`].
    FileRemove,
    /// Reply to [`M2dirYield::WantsRename`].
    Rename,
}
