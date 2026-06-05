//! # Standard, blocking m2dir client
//!
//! Holds a single filesystem root and exposes one method per common
//! coroutine. Every method runs its coroutine to completion through
//! [`M2dirClient::run`] by servicing each [`M2dirYield`] request via
//! [`std::fs`].
//!
//! [`M2dirYield`]: crate::coroutine::M2dirYield

use std::{
    collections::{BTreeMap, BTreeSet, hash_map::RandomState},
    fs,
    hash::{BuildHasher, Hasher},
    io,
    path::{Path, PathBuf},
    process,
    string::ToString,
    thread,
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{
    coroutine::*,
    entry::{
        delete::*,
        get::*,
        list::*,
        store::*,
        types::{M2dirEntry, M2dirFullEntry, ParseFilenameError},
        utils::validate_checksum,
    },
    flag::types::M2dirFlags,
    flag::{add::*, remove::*, set::*},
    m2dir::{
        create::*,
        delete::*,
        list::*,
        types::{DOT_M2DIR, LoadM2dirError, M2dir},
    },
    path::M2dirPath,
    store::{DOT_M2STORE, M2dirStore, M2dirStoreError},
};

/// Errors returned by [`M2dirClient`].
#[derive(Debug, Error)]
pub enum M2dirClientError {
    #[error(transparent)]
    Store(#[from] M2dirStoreError),
    #[error(transparent)]
    LoadM2dir(#[from] LoadM2dirError),
    #[error(transparent)]
    CreateM2dir(#[from] M2dirCreateError),
    #[error(transparent)]
    DeleteM2dir(#[from] M2dirDeleteError),
    #[error(transparent)]
    ListM2dirs(#[from] M2dirListError),
    #[error(transparent)]
    ListEntries(#[from] M2dirEntryListError),
    #[error(transparent)]
    GetEntry(#[from] M2dirEntryGetError),
    #[error(transparent)]
    StoreEntry(#[from] M2dirEntryStoreError),
    #[error(transparent)]
    DeleteEntry(#[from] M2dirEntryDeleteError),
    #[error(transparent)]
    AddFlags(#[from] M2dirFlagAddError),
    #[error(transparent)]
    RemoveFlags(#[from] M2dirFlagRemoveError),
    #[error(transparent)]
    SetFlags(#[from] M2dirFlagSetError),
    #[error(transparent)]
    Parse(#[from] ParseFilenameError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Std-blocking m2dir client wrapping a filesystem root.
///
/// The root must point to an m2store: a directory containing a
/// `.m2store` marker. M2dir helpers resolve folder names against
/// this root.
#[derive(Debug)]
pub struct M2dirClient {
    root: M2dirPath,
}

impl M2dirClient {
    /// Builds a client rooted at `root`. No filesystem check is
    /// performed at construction time.
    pub fn new(root: impl Into<M2dirPath>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the filesystem root this client operates on.
    pub fn root(&self) -> &M2dirPath {
        &self.root
    }

    /// Drives any standard-shape coroutine (`Yield = M2dirYield`,
    /// `Return = Result<Output, Error>`) against the local
    /// filesystem until it terminates.
    pub fn run<C, T, E>(&self, mut coroutine: C) -> Result<T, M2dirClientError>
    where
        C: M2dirCoroutine<Yield = M2dirYield, Return = Result<T, E>>,
        M2dirClientError: From<E>,
    {
        let mut arg: Option<M2dirArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirCoroutineState::Complete(Ok(out)) => return Ok(out),
                M2dirCoroutineState::Complete(Err(err)) => return Err(err.into()),
                M2dirCoroutineState::Yielded(M2dirYield::WantsPid) => {
                    arg = Some(M2dirArg::Pid(process::id()));
                }
                M2dirCoroutineState::Yielded(M2dirYield::WantsRandom { len }) => {
                    arg = Some(M2dirArg::Random(random_bytes(len)));
                }
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileExists(paths)) => {
                    arg = Some(M2dirArg::FileExists(file_exists(paths)));
                }
                M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths)) => {
                    arg = Some(M2dirArg::DirRead(read_dirs(paths)?));
                }
                M2dirCoroutineState::Yielded(M2dirYield::WantsDirCreate(paths)) => {
                    create_dirs(paths)?;
                    arg = Some(M2dirArg::DirCreate);
                }
                M2dirCoroutineState::Yielded(M2dirYield::WantsDirRemove(paths)) => {
                    remove_dirs(paths)?;
                    arg = Some(M2dirArg::DirRemove);
                }
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileRead(paths)) => {
                    arg = Some(M2dirArg::FileRead(read_files_tolerant(paths)?));
                }
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files)) => {
                    write_files(files)?;
                    arg = Some(M2dirArg::FileCreate);
                }
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileRemove(paths)) => {
                    remove_files_tolerant(paths)?;
                    arg = Some(M2dirArg::FileRemove);
                }
                M2dirCoroutineState::Yielded(M2dirYield::WantsRename(pairs)) => {
                    rename_paths(pairs)?;
                    arg = Some(M2dirArg::Rename);
                }
            }
        }
    }

    /// Opens the m2store at the client root, returning a typed
    /// handle on success.
    pub fn open_store(&self) -> Result<M2dirStore, M2dirClientError> {
        load_store(self.root.clone()).map_err(Into::into)
    }

    /// Initialises a brand new m2store at the client root: creates
    /// the directory if needed and writes the `.m2store` marker.
    pub fn init_store(&self) -> Result<M2dirStore, M2dirClientError> {
        trace!("init m2store at {}", self.root);

        fs::create_dir_all(self.root.as_str())?;
        let marker = self.root.join(DOT_M2STORE);
        if !Path::new(marker.as_str()).exists() {
            fs::write(marker.as_str(), b"")?;
        }

        Ok(M2dirStore::from_path(self.root.clone()))
    }

    /// Opens an existing m2dir at `path`, validating the `.m2dir`
    /// marker.
    pub fn open_m2dir(&self, path: impl Into<M2dirPath>) -> Result<M2dir, M2dirClientError> {
        load_m2dir(path.into()).map_err(Into::into)
    }

    // ---- M2dir lifecycle ----------------------------------------

    /// Creates the m2dir folder `name` and writes the `.m2dir`
    /// marker.
    pub fn create_m2dir(&self, name: &str) -> Result<M2dir, M2dirClientError> {
        let store = self.open_store()?;
        let coroutine = M2dirCreate::new(&store, name, M2dirCreateOptions::default())?;
        self.run(coroutine)
    }

    /// Recursively removes the m2dir at `path`.
    pub fn delete_m2dir(&self, path: impl Into<M2dirPath>) -> Result<(), M2dirClientError> {
        self.run(M2dirDelete::new(path, M2dirDeleteOptions::default()))
    }

    /// Lists every m2dir under the store root.
    pub fn list_m2dirs(&self) -> Result<BTreeSet<M2dir>, M2dirClientError> {
        let store = self.open_store()?;
        self.run(M2dirList::new(&store, M2dirListOptions::default()))
    }

    // ---- Entries ------------------------------------------------

    /// Lists every entry inside `m2dir`.
    pub fn list_entries(&self, m2dir: M2dir) -> Result<Vec<M2dirEntry>, M2dirClientError> {
        self.run(M2dirEntryList::new(m2dir, M2dirEntryListOptions::default()))
    }

    /// Reads the file backing `entry` and validates its checksum.
    ///
    /// Prefer this over [`Self::get`] when the entry is already known:
    /// skips the directory scan used to resolve an id.
    pub fn read_entry(&self, entry: &M2dirEntry) -> Result<Vec<u8>, M2dirClientError> {
        let path = entry.path();
        trace!("read entry at {path}");

        let bytes = fs::read(path.as_str())?;
        let checksum = entry.checksum();

        if !validate_checksum(checksum, &bytes) {
            return Err(ParseFilenameError::InvalidChecksum {
                path: path.clone(),
                expected: checksum.to_string(),
                got: entry.id().to_string(),
            }
            .into());
        }

        Ok(bytes)
    }

    /// Reads the bytes and flags of every entry sequentially.
    ///
    /// Returns an unordered set: callers that need a specific order
    /// must sort the collected entries themselves. Use
    /// [`Self::read_entries_par`] for the parallel variant.
    pub fn read_entries(
        &self,
        m2dir: &M2dir,
        entries: &[M2dirEntry],
    ) -> Result<BTreeSet<M2dirFullEntry>, M2dirClientError> {
        entries
            .iter()
            .map(|entry| self.read_full_entry(m2dir, entry))
            .collect()
    }

    /// Parallel variant of [`Self::read_entries`] backed by a
    /// `std::thread::scope` worker pool sized to
    /// [`thread::available_parallelism`].
    pub fn read_entries_par(
        &self,
        m2dir: &M2dir,
        entries: &[M2dirEntry],
    ) -> Result<BTreeSet<M2dirFullEntry>, M2dirClientError> {
        if entries.len() <= 1 {
            return entries
                .iter()
                .map(|entry| self.read_full_entry(m2dir, entry))
                .collect();
        }

        let n_threads = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
            .min(entries.len());

        let chunk_size = entries.len().div_ceil(n_threads);

        thread::scope(|s| -> Result<BTreeSet<M2dirFullEntry>, M2dirClientError> {
            let mut handles = Vec::with_capacity(n_threads);

            for chunk in entries.chunks(chunk_size) {
                let this = self;

                handles.push(s.spawn(move || {
                    chunk
                        .iter()
                        .map(|entry| this.read_full_entry(m2dir, entry))
                        .collect::<Result<Vec<_>, _>>()
                }));
            }

            let mut out = BTreeSet::new();

            for handle in handles {
                for full in handle.join().expect("m2dir worker thread panicked")? {
                    out.insert(full);
                }
            }

            Ok(out)
        })
    }

    fn read_full_entry(
        &self,
        m2dir: &M2dir,
        entry: &M2dirEntry,
    ) -> Result<M2dirFullEntry, M2dirClientError> {
        let contents = self.read_entry(entry)?;
        let flags = self.read_flags(m2dir, entry.id())?;

        Ok(M2dirFullEntry::from_parts(entry.clone(), contents, flags))
    }

    /// Locates and reads entry `id` from `m2dir`, validating the
    /// checksum embedded in the filename.
    pub fn get(
        &self,
        m2dir: M2dir,
        id: impl ToString,
    ) -> Result<(M2dirEntry, Vec<u8>), M2dirClientError> {
        let M2dirEntryGetOutput { entry, contents } = self.run(M2dirEntryGet::new(
            m2dir,
            id,
            M2dirEntryGetOptions::default(),
        ))?;
        Ok((entry, contents))
    }

    /// Writes `bytes` to a temporary file inside `m2dir`, then
    /// atomically renames it to its checksum-based final filename.
    pub fn store(&self, m2dir: M2dir, bytes: Vec<u8>) -> Result<M2dirEntry, M2dirClientError> {
        self.run(M2dirEntryStore::new(
            m2dir,
            bytes,
            M2dirEntryStoreOptions::default(),
        ))
    }

    /// Removes entry `id` and every matching `.meta/<id>*` file.
    pub fn delete_entry(&self, m2dir: M2dir, id: impl ToString) -> Result<(), M2dirClientError> {
        self.run(M2dirEntryDelete::new(
            m2dir,
            id,
            M2dirEntryDeleteOptions::default(),
        ))
    }

    // ---- Flags --------------------------------------------------

    /// Reads the `.flags` metadata file for entry `id` inside
    /// `m2dir`, returning an empty set if the file is missing.
    pub fn read_flags(
        &self,
        m2dir: &M2dir,
        id: impl AsRef<str>,
    ) -> Result<M2dirFlags, M2dirClientError> {
        let path = m2dir.flags_path(id.as_ref());
        trace!("read flags at {path}");

        match fs::read_to_string(path.as_str()) {
            Ok(text) => Ok(M2dirFlags::from_meta(&text)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(M2dirFlags::default()),
            Err(err) => Err(err.into()),
        }
    }

    /// Adds `flags` to entry `id`'s flags metadata file.
    pub fn add_flags(
        &self,
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: M2dirFlags,
    ) -> Result<(), M2dirClientError> {
        self.run(M2dirFlagAdd::new(
            m2dir,
            id,
            flags,
            M2dirFlagAddOptions::default(),
        ))
    }

    /// Removes `flags` from entry `id`'s flags metadata file. When
    /// the resulting set is empty the file is deleted.
    pub fn remove_flags(
        &self,
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: M2dirFlags,
    ) -> Result<(), M2dirClientError> {
        self.run(M2dirFlagRemove::new(
            m2dir,
            id,
            flags,
            M2dirFlagRemoveOptions::default(),
        ))
    }

    /// Replaces entry `id`'s flags metadata file with `flags`,
    /// deleting it when `flags` is empty.
    pub fn set_flags(
        &self,
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: M2dirFlags,
    ) -> Result<(), M2dirClientError> {
        self.run(M2dirFlagSet::new(
            m2dir,
            id,
            flags,
            M2dirFlagSetOptions::default(),
        ))
    }
}

// ---- Loaders -----------------------------------------------------

fn load_store(path: M2dirPath) -> Result<M2dirStore, M2dirStoreError> {
    if !Path::new(path.as_str()).is_dir() {
        return Err(M2dirStoreError::NotDir(path));
    }

    let marker = path.join(DOT_M2STORE);
    if !Path::new(marker.as_str()).exists() {
        return Err(M2dirStoreError::NoDotM2store(path));
    }

    Ok(M2dirStore::from_path(path))
}

fn load_m2dir(path: M2dirPath) -> Result<M2dir, LoadM2dirError> {
    if !Path::new(path.as_str()).is_dir() {
        return Err(LoadM2dirError::NotDir(path));
    }

    let marker = path.join(DOT_M2DIR);
    if !Path::new(marker.as_str()).exists() {
        return Err(LoadM2dirError::NoDotM2dir(path));
    }

    Ok(M2dir::from_path(path))
}

// ---- Path normalization -----------------------------------------

fn normalize_path(path: PathBuf) -> M2dirPath {
    let s = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    let s = s.replace('\\', "/");
    M2dirPath::new(s)
}

// ---- Filesystem helpers -----------------------------------------

fn create_dirs(paths: BTreeSet<M2dirPath>) -> Result<(), io::Error> {
    for path in paths {
        trace!("create_dir_all {path}");
        fs::create_dir_all(path.as_str())?;
    }
    Ok(())
}

fn remove_dirs(paths: BTreeSet<M2dirPath>) -> Result<(), io::Error> {
    for path in paths {
        trace!("remove_dir_all {path}");
        fs::remove_dir_all(path.as_str())?;
    }
    Ok(())
}

fn write_files(files: BTreeMap<M2dirPath, Vec<u8>>) -> Result<(), io::Error> {
    for (path, contents) in files {
        trace!("write {path} ({} bytes)", contents.len());

        if let Some(parent) = Path::new(path.as_str()).parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path.as_str(), &contents)?;
    }
    Ok(())
}

fn remove_files_tolerant(paths: BTreeSet<M2dirPath>) -> Result<(), io::Error> {
    for path in paths {
        trace!("remove_file (tolerant) {path}");
        match fs::remove_file(path.as_str()) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn read_dirs(
    paths: BTreeSet<M2dirPath>,
) -> Result<BTreeMap<M2dirPath, BTreeSet<M2dirPath>>, io::Error> {
    let mut entries = BTreeMap::new();

    for path in paths {
        trace!("read_dir {path}");

        let mut names = BTreeSet::new();
        match fs::read_dir(path.as_str()) {
            Ok(iter) => {
                for entry in iter {
                    let entry = entry?;
                    names.insert(normalize_path(entry.path()));
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) if err.kind() == io::ErrorKind::NotADirectory => {}
            Err(err) => return Err(err),
        }

        entries.insert(path, names);
    }

    Ok(entries)
}

fn read_files_tolerant(
    paths: BTreeSet<M2dirPath>,
) -> Result<BTreeMap<M2dirPath, Vec<u8>>, io::Error> {
    let mut contents = BTreeMap::new();

    for path in paths {
        trace!("read_file (tolerant) {path}");
        match fs::read(path.as_str()) {
            Ok(bytes) => {
                contents.insert(path, bytes);
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                contents.insert(path, Vec::new());
            }
            Err(err) => return Err(err),
        }
    }

    Ok(contents)
}

fn rename_paths(pairs: Vec<(M2dirPath, M2dirPath)>) -> Result<(), io::Error> {
    for (from, to) in pairs {
        trace!("rename {from} -> {to}");
        fs::rename(from.as_str(), to.as_str())?;
    }
    Ok(())
}

fn file_exists(paths: BTreeSet<M2dirPath>) -> BTreeMap<M2dirPath, bool> {
    let mut out = BTreeMap::new();
    for path in paths {
        let exists = fs::metadata(path.as_str())
            .map(|m| m.is_file())
            .unwrap_or(false);
        trace!("file_exists {path}: {exists}");
        out.insert(path, exists);
    }
    out
}

// ---- Entropy ----------------------------------------------------

/// Generates `len` pseudo-random bytes seeded from
/// [`RandomState`], iterated via xorshift64*.
fn random_bytes(len: usize) -> Vec<u8> {
    let mut state = RandomState::new().build_hasher().finish();
    if state == 0 {
        state = 0xdeadbeef;
    }

    let mut out = Vec::with_capacity(len);
    let mut buf = 0u64;
    let mut i = 8;

    while out.len() < len {
        if i == 8 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            buf = state;
            i = 0;
        }
        out.push(buf as u8);
        buf >>= 8;
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::tempdir;

    use crate::{client::*, flag::types::M2dirFlags, store::DOT_M2STORE};

    fn client() -> (tempfile::TempDir, M2dirClient) {
        let dir = tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let client = M2dirClient::new(root);
        client.init_store().unwrap();
        (dir, client)
    }

    #[test]
    fn init_store_writes_marker() {
        let (dir, _client) = client();
        assert!(dir.path().join(DOT_M2STORE).exists());
    }

    #[test]
    fn create_m2dir_writes_marker() {
        let (_dir, client) = client();

        let inbox = client.create_m2dir("inbox").unwrap();
        assert!(Path::new(inbox.path().as_str()).is_dir());
        assert!(Path::new(inbox.marker_path().as_str()).exists());
        assert!(Path::new(inbox.meta_dir().as_str()).is_dir());
    }

    #[test]
    fn list_m2dirs_finds_created_folder() {
        let (_dir, client) = client();

        client.create_m2dir("inbox").unwrap();
        client.create_m2dir("sent").unwrap();

        let m2dirs = client.list_m2dirs().unwrap();
        assert_eq!(m2dirs.len(), 2);
    }

    #[test]
    fn store_and_list_entries_round_trip() {
        let (_dir, client) = client();

        let inbox = client.create_m2dir("inbox").unwrap();
        let msg = b"From: alice@example.org\r\nDate: Tue, 15 Apr 1994 08:12:31 GMT\r\nSubject: hi\r\n\r\nbody\r\n";

        let entry = client.store(inbox.clone(), msg.to_vec()).unwrap();
        assert!(Path::new(entry.path().as_str()).is_file());

        let listed = client.list_entries(inbox.clone()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id(), entry.id());

        let (fetched, contents) = client.get(inbox, entry.id()).unwrap();
        assert_eq!(fetched.id(), entry.id());
        assert_eq!(contents, msg);
    }

    #[test]
    fn flags_round_trip_via_meta() {
        let (_dir, client) = client();

        let inbox = client.create_m2dir("inbox").unwrap();
        let msg = b"From: a\r\n\r\nbody\r\n";
        let entry = client.store(inbox.clone(), msg.to_vec()).unwrap();

        let initial = client.read_flags(&inbox, entry.id()).unwrap();
        assert_eq!(initial.len(), 0);

        let mut to_add = M2dirFlags::default();
        to_add.insert("$seen");
        to_add.insert("$forwarded");
        client.add_flags(&inbox, entry.id(), to_add).unwrap();

        let after_add = client.read_flags(&inbox, entry.id()).unwrap();
        assert_eq!(after_add.len(), 2);
        assert!(after_add.contains("$seen"));
        assert!(after_add.contains("$forwarded"));

        let mut to_remove = M2dirFlags::default();
        to_remove.insert("$seen");
        client.remove_flags(&inbox, entry.id(), to_remove).unwrap();

        let after_remove = client.read_flags(&inbox, entry.id()).unwrap();
        assert_eq!(after_remove.len(), 1);
        assert!(after_remove.contains("$forwarded"));

        let mut replacement = M2dirFlags::default();
        replacement.insert("custom");
        replacement.insert("$junk");
        client.set_flags(&inbox, entry.id(), replacement).unwrap();

        let after_set = client.read_flags(&inbox, entry.id()).unwrap();
        assert_eq!(after_set.len(), 2);
        assert!(after_set.contains("custom"));
        assert!(after_set.contains("$junk"));

        client
            .set_flags(&inbox, entry.id(), M2dirFlags::default())
            .unwrap();
        let after_clear = client.read_flags(&inbox, entry.id()).unwrap();
        assert!(after_clear.is_empty());
        assert!(!Path::new(inbox.flags_path(entry.id()).as_str()).exists());
    }

    #[test]
    fn delete_entry_removes_file_and_flags_meta() {
        let (_dir, client) = client();

        let inbox = client.create_m2dir("inbox").unwrap();
        let entry = client.store(inbox.clone(), b"hello".to_vec()).unwrap();

        let mut flags = M2dirFlags::default();
        flags.insert("$seen");
        client.add_flags(&inbox, entry.id(), flags).unwrap();
        assert!(Path::new(inbox.flags_path(entry.id()).as_str()).exists());

        client.delete_entry(inbox.clone(), entry.id()).unwrap();
        assert!(!Path::new(entry.path().as_str()).exists());
        assert!(!Path::new(inbox.flags_path(entry.id()).as_str()).exists());

        let listed = client.list_entries(inbox).unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn delete_m2dir_removes_tree() {
        let (_dir, client) = client();

        let inbox = client.create_m2dir("inbox").unwrap();
        let path = inbox.path().clone();
        assert!(Path::new(path.as_str()).is_dir());

        client.delete_m2dir(path.clone()).unwrap();
        assert!(!Path::new(path.as_str()).exists());
    }
}
