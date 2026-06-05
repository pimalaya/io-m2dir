# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added the `M2dirCoroutine` trait mirroring `core::ops::Coroutine`.

  Composed of `Yield` and `Return` associated types plus a two-variant `M2dirCoroutineState<Y, R>` (`Yielded(Y)` / `Complete(R)`). Every coroutine picks the shared `M2dirYield` enum (filesystem `Wants*` requests plus `WantsPid` / `WantsRandom`) and is fed back via the matching `M2dirArg` enum.

- Added I/O-free `M2dirCreate` coroutine.

  Creates the m2dir folder, its `.meta` sub-directory and the `.m2dir` marker file.

- Added I/O-free `M2dirDelete` coroutine.

  Recursively removes an m2dir.

- Added I/O-free `M2dirList` coroutine.

  Walks the m2store tree depth-first and surfaces every directory carrying a `.m2dir` marker; nested m2dirs are supported.

- Added I/O-free `M2dirEntryStore` coroutine.

  Follows the m2dir delivery protocol: writes to a temporary file first, then atomically renames into `<date>,<checksum>.<nonce>`.

- Added I/O-free `M2dirEntryGet` coroutine.

  Locates an entry by id and validates the checksum embedded in the filename against the file contents.

- Added I/O-free `M2dirEntryList` coroutine.

  Returns every confirmed entry inside an m2dir, skipping dotfiles and filenames that do not match the spec.

- Added I/O-free `M2dirEntryDelete` coroutine.

  Removes the entry file and every sibling `.meta/<id>*` metadata file.

- Added I/O-free `M2dirFlagAdd` coroutine.

  Merges flags into the `.meta/<id>.flags` sidecar (one flag per line, deterministic alphabetical order).

- Added I/O-free `M2dirFlagRemove` coroutine.

  Subtracts flags from the sidecar and deletes the file when the remainder is empty.

- Added I/O-free `M2dirFlagSet` coroutine.

  Replaces the sidecar, or deletes it when the new set is empty.

- Added the `M2dirPath` / `M2dirStore` / `M2dir` type hierarchy.

  `M2dirPath` is the `/`-separated path used by both m2dir and m2store. `M2dirStore` wraps the store root and resolves logical folder names to on-disk paths via the m2dir spec's percent-encoding. `M2dir` wraps a single m2dir directory and derives entry / metadata / temporary filenames.

- Added the `client` cargo feature (default) enabling `M2dirClient`.

  Standard, blocking client backed by `std::fs` that drives any standard-Yield coroutine to completion, with `read_entries_par` for parallel reads via `std::thread::scope`. When the feature is off the `Date:` header parser falls back to a private no_std implementation so date prefixes are still computed in pure no_std builds.

[unreleased]: https://github.com/pimalaya/io-m2dir/compare/root..HEAD
