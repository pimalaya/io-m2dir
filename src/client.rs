//! Standard, blocking m2dir client.
//!
//! Holds a single filesystem root and exposes one method per
//! coroutine. Every method runs its coroutine to completion by
//! performing the requested filesystem operations via [`std::fs`]
//! in a resume loop.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::ToString,
    vec::Vec,
};
use std::{
    collections::hash_map::RandomState,
    fs,
    hash::{BuildHasher, Hasher},
    io,
    path::{Path, PathBuf},
    process, thread,
};

use log::trace;
use thiserror::Error;

use crate::{
    coroutines::{
        flag_add::*, flag_remove::*, flag_set::*, mailbox_create::*, mailbox_delete::*,
        mailbox_list::*, message_delete::*, message_get::*, message_list::*, message_store::*,
    },
    entry::{M2dirEntry, M2dirFullEntry, ParseFilenameError, validate_checksum},
    flag::M2dirFlags,
    m2dir::{DOT_M2DIR, LoadM2dirError, M2dir},
    m2store::{DOT_M2STORE, LoadM2storeError, M2store, NewFolderError},
    path::M2dirPath,
};

/// Errors returned by [`M2dirClient`].
#[derive(Debug, Error)]
pub enum M2dirClientError {
    #[error(transparent)]
    LoadM2store(#[from] LoadM2storeError),
    #[error(transparent)]
    LoadM2dir(#[from] LoadM2dirError),
    #[error(transparent)]
    NewFolder(#[from] NewFolderError),
    #[error(transparent)]
    CreateMailbox(#[from] M2dirMailboxCreateError),
    #[error(transparent)]
    DeleteMailbox(#[from] M2dirMailboxDeleteError),
    #[error(transparent)]
    ListMailboxes(#[from] M2dirMailboxListError),
    #[error(transparent)]
    ListMessages(#[from] M2dirMessageListError),
    #[error(transparent)]
    GetMessage(#[from] M2dirMessageGetError),
    #[error(transparent)]
    StoreMessage(#[from] M2dirMessageStoreError),
    #[error(transparent)]
    DeleteMessage(#[from] M2dirMessageDeleteError),
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
/// `.m2store` marker. Mailbox helpers resolve folder names against
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

    /// Opens the m2store at the client root, returning a typed
    /// handle on success.
    pub fn open_store(&self) -> Result<M2store, M2dirClientError> {
        load_m2store(self.root.clone()).map_err(Into::into)
    }

    /// Initialises a brand new m2store at the client root: creates
    /// the directory if needed and writes the `.m2store` marker.
    pub fn init_store(&self) -> Result<M2store, M2dirClientError> {
        trace!("init m2store at {}", self.root);

        fs::create_dir_all(self.root.as_str())?;
        let marker = self.root.join(DOT_M2STORE);
        if !Path::new(marker.as_str()).exists() {
            fs::write(marker.as_str(), b"")?;
        }

        Ok(M2store::from_path(self.root.clone()))
    }

    /// Opens an existing m2dir at `path`, validating the `.m2dir`
    /// marker.
    pub fn open_m2dir(&self, path: impl Into<M2dirPath>) -> Result<M2dir, M2dirClientError> {
        load_m2dir(path.into()).map_err(Into::into)
    }

    // ---- Mailbox lifecycle --------------------------------------

    /// Creates the m2dir folder `name` and writes the `.m2dir`
    /// marker.
    pub fn create_mailbox(&self, name: &str) -> Result<M2dir, M2dirClientError> {
        let store = self.open_store()?;
        let mut coroutine = M2dirMailboxCreate::new(&store, name)?;
        let mut arg: Option<M2dirMailboxCreateArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirMailboxCreateResult::Ok(m2dir) => return Ok(m2dir),
                M2dirMailboxCreateResult::WantsDirCreate(paths) => {
                    create_dirs(paths)?;
                    arg = Some(M2dirMailboxCreateArg::DirCreate);
                }
                M2dirMailboxCreateResult::WantsFileCreate(files) => {
                    write_files(files)?;
                    arg = Some(M2dirMailboxCreateArg::FileCreate);
                }
                M2dirMailboxCreateResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Recursively removes the m2dir at `path`.
    pub fn delete_mailbox(&self, path: impl Into<M2dirPath>) -> Result<(), M2dirClientError> {
        let mut coroutine = M2dirMailboxDelete::new(path);
        let mut arg: Option<M2dirMailboxDeleteArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirMailboxDeleteResult::Ok => return Ok(()),
                M2dirMailboxDeleteResult::WantsDirRemove(paths) => {
                    remove_dirs(paths)?;
                    arg = Some(M2dirMailboxDeleteArg::DirRemove);
                }
                M2dirMailboxDeleteResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Lists every m2dir under the store root.
    pub fn list_mailboxes(&self) -> Result<BTreeSet<M2dir>, M2dirClientError> {
        let store = self.open_store()?;
        let mut coroutine = M2dirMailboxList::new(&store);
        let mut arg: Option<M2dirMailboxListArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirMailboxListResult::Ok(found) => return Ok(found),
                M2dirMailboxListResult::WantsDirRead(paths) => {
                    let entries = read_dirs(paths)?;
                    arg = Some(M2dirMailboxListArg::DirRead(entries));
                }
                M2dirMailboxListResult::WantsFileExists(paths) => {
                    let probes = file_exists(paths);
                    arg = Some(M2dirMailboxListArg::FileExists(probes));
                }
                M2dirMailboxListResult::Err(err) => return Err(err.into()),
            }
        }
    }

    // ---- Messages -----------------------------------------------

    /// Lists every entry inside `m2dir`.
    pub fn list_entries(&self, m2dir: M2dir) -> Result<Vec<M2dirEntry>, M2dirClientError> {
        let mut coroutine = M2dirMessageList::new(m2dir);
        let mut arg: Option<M2dirMessageListArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirMessageListResult::Ok(entries) => return Ok(entries),
                M2dirMessageListResult::WantsDirRead(paths) => {
                    let entries = read_dirs(paths)?;
                    arg = Some(M2dirMessageListArg::DirRead(entries));
                }
                M2dirMessageListResult::WantsFileExists(paths) => {
                    let probes = file_exists(paths);
                    arg = Some(M2dirMessageListArg::FileExists(probes));
                }
                M2dirMessageListResult::Err(err) => return Err(err.into()),
            }
        }
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
        let mut coroutine = M2dirMessageGet::new(m2dir, id);
        let mut arg: Option<M2dirMessageGetArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirMessageGetResult::Ok { entry, contents } => return Ok((entry, contents)),
                M2dirMessageGetResult::WantsDirRead(paths) => {
                    let entries = read_dirs(paths)?;
                    arg = Some(M2dirMessageGetArg::DirRead(entries));
                }
                M2dirMessageGetResult::WantsFileExists(paths) => {
                    let probes = file_exists(paths);
                    arg = Some(M2dirMessageGetArg::FileExists(probes));
                }
                M2dirMessageGetResult::WantsFileRead(paths) => {
                    let files = read_files(paths)?;
                    arg = Some(M2dirMessageGetArg::FileRead(files));
                }
                M2dirMessageGetResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Writes `bytes` to a temporary file inside `m2dir`, then
    /// atomically renames it to its checksum-based final filename.
    pub fn store(&self, m2dir: M2dir, bytes: Vec<u8>) -> Result<M2dirEntry, M2dirClientError> {
        let mut coroutine = M2dirMessageStore::new(m2dir, bytes);
        let mut arg: Option<M2dirMessageStoreArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirMessageStoreResult::Ok(entry) => return Ok(entry),
                M2dirMessageStoreResult::WantsPid => {
                    arg = Some(M2dirMessageStoreArg::Pid(process::id()));
                }
                M2dirMessageStoreResult::WantsRandom { len } => {
                    arg = Some(M2dirMessageStoreArg::Random(random_bytes(len)));
                }
                M2dirMessageStoreResult::WantsFileCreate(files) => {
                    write_files(files)?;
                    arg = Some(M2dirMessageStoreArg::FileCreate);
                }
                M2dirMessageStoreResult::WantsRename(pairs) => {
                    rename_paths(pairs)?;
                    arg = Some(M2dirMessageStoreArg::Rename);
                }
                M2dirMessageStoreResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Removes entry `id` and every matching `.meta/<id>*` file.
    pub fn delete_message(&self, m2dir: M2dir, id: impl ToString) -> Result<(), M2dirClientError> {
        let mut coroutine = M2dirMessageDelete::new(m2dir, id);
        let mut arg: Option<M2dirMessageDeleteArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirMessageDeleteResult::Ok => return Ok(()),
                M2dirMessageDeleteResult::WantsDirRead(paths) => {
                    let entries = read_dirs(paths)?;
                    arg = Some(M2dirMessageDeleteArg::DirRead(entries));
                }
                M2dirMessageDeleteResult::WantsFileExists(paths) => {
                    let probes = file_exists(paths);
                    arg = Some(M2dirMessageDeleteArg::FileExists(probes));
                }
                M2dirMessageDeleteResult::WantsFileRemove(paths) => {
                    remove_files(paths)?;
                    arg = Some(M2dirMessageDeleteArg::FileRemove);
                }
                M2dirMessageDeleteResult::Err(err) => return Err(err.into()),
            }
        }
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
        let mut coroutine = M2dirFlagAdd::new(m2dir, id, flags);
        let mut arg: Option<M2dirFlagAddArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirFlagAddResult::Ok => return Ok(()),
                M2dirFlagAddResult::WantsFileRead(paths) => {
                    let files = read_files_tolerant(paths)?;
                    arg = Some(M2dirFlagAddArg::FileRead(files));
                }
                M2dirFlagAddResult::WantsFileCreate(files) => {
                    write_files(files)?;
                    arg = Some(M2dirFlagAddArg::FileCreate);
                }
                M2dirFlagAddResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Removes `flags` from entry `id`'s flags metadata file. When
    /// the resulting set is empty the file is deleted.
    pub fn remove_flags(
        &self,
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: M2dirFlags,
    ) -> Result<(), M2dirClientError> {
        let mut coroutine = M2dirFlagRemove::new(m2dir, id, flags);
        let mut arg: Option<M2dirFlagRemoveArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirFlagRemoveResult::Ok => return Ok(()),
                M2dirFlagRemoveResult::WantsFileRead(paths) => {
                    let files = read_files_tolerant(paths)?;
                    arg = Some(M2dirFlagRemoveArg::FileRead(files));
                }
                M2dirFlagRemoveResult::WantsFileCreate(files) => {
                    write_files(files)?;
                    arg = Some(M2dirFlagRemoveArg::FileCreate);
                }
                M2dirFlagRemoveResult::WantsFileRemove(paths) => {
                    remove_files(paths)?;
                    arg = Some(M2dirFlagRemoveArg::FileRemove);
                }
                M2dirFlagRemoveResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Replaces entry `id`'s flags metadata file with `flags`, deleting it when
    /// `flags` is empty.
    pub fn set_flags(
        &self,
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: M2dirFlags,
    ) -> Result<(), M2dirClientError> {
        let mut coroutine = M2dirFlagSet::new(m2dir, id, flags);
        let mut arg: Option<M2dirFlagSetArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                M2dirFlagSetResult::Ok => return Ok(()),
                M2dirFlagSetResult::WantsFileCreate(files) => {
                    write_files(files)?;
                    arg = Some(M2dirFlagSetArg::FileCreate);
                }
                M2dirFlagSetResult::WantsFileRemove(paths) => {
                    remove_files_tolerant(paths)?;
                    arg = Some(M2dirFlagSetArg::FileRemove);
                }
                M2dirFlagSetResult::Err(err) => return Err(err.into()),
            }
        }
    }
}

// ---- Loaders -----------------------------------------------------

fn load_m2store(path: M2dirPath) -> Result<M2store, LoadM2storeError> {
    if !Path::new(path.as_str()).is_dir() {
        return Err(LoadM2storeError::NotDir(path));
    }

    let marker = path.join(DOT_M2STORE);
    if !Path::new(marker.as_str()).exists() {
        return Err(LoadM2storeError::NoDotM2store(path));
    }

    Ok(M2store::from_path(path))
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

fn remove_files(paths: BTreeSet<M2dirPath>) -> Result<(), io::Error> {
    for path in paths {
        trace!("remove_file {path}");
        fs::remove_file(path.as_str())?;
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

fn read_files(paths: BTreeSet<M2dirPath>) -> Result<BTreeMap<M2dirPath, Vec<u8>>, io::Error> {
    let mut contents = BTreeMap::new();

    for path in paths {
        trace!("read_file {path}");
        let bytes = fs::read(path.as_str())?;
        contents.insert(path, bytes);
    }

    Ok(contents)
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

    use crate::client::*;

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
    fn create_mailbox_writes_dot_m2dir() {
        let (_dir, client) = client();

        let inbox = client.create_mailbox("inbox").unwrap();
        assert!(Path::new(inbox.path().as_str()).is_dir());
        assert!(Path::new(inbox.marker_path().as_str()).exists());
        assert!(Path::new(inbox.meta_dir().as_str()).is_dir());
    }

    #[test]
    fn list_mailboxes_finds_created_folder() {
        let (_dir, client) = client();

        client.create_mailbox("inbox").unwrap();
        client.create_mailbox("sent").unwrap();

        let mailboxes = client.list_mailboxes().unwrap();
        assert_eq!(mailboxes.len(), 2);
    }

    #[test]
    fn store_and_list_entries_round_trip() {
        let (_dir, client) = client();

        let inbox = client.create_mailbox("inbox").unwrap();
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

        let inbox = client.create_mailbox("inbox").unwrap();
        let msg = b"From: a\r\n\r\nbody\r\n";
        let entry = client.store(inbox.clone(), msg.to_vec()).unwrap();

        let initial = client.read_flags(&inbox, entry.id()).unwrap();
        assert_eq!(initial.len(), 0);

        let mut to_add = crate::flag::M2dirFlags::default();
        to_add.insert("$seen");
        to_add.insert("$forwarded");
        client.add_flags(&inbox, entry.id(), to_add).unwrap();

        let after_add = client.read_flags(&inbox, entry.id()).unwrap();
        assert_eq!(after_add.len(), 2);
        assert!(after_add.contains("$seen"));
        assert!(after_add.contains("$forwarded"));

        let mut to_remove = crate::flag::M2dirFlags::default();
        to_remove.insert("$seen");
        client.remove_flags(&inbox, entry.id(), to_remove).unwrap();

        let after_remove = client.read_flags(&inbox, entry.id()).unwrap();
        assert_eq!(after_remove.len(), 1);
        assert!(after_remove.contains("$forwarded"));

        let mut replacement = crate::flag::M2dirFlags::default();
        replacement.insert("custom");
        replacement.insert("$junk");
        client.set_flags(&inbox, entry.id(), replacement).unwrap();

        let after_set = client.read_flags(&inbox, entry.id()).unwrap();
        assert_eq!(after_set.len(), 2);
        assert!(after_set.contains("custom"));
        assert!(after_set.contains("$junk"));

        client
            .set_flags(&inbox, entry.id(), crate::flag::M2dirFlags::default())
            .unwrap();
        let after_clear = client.read_flags(&inbox, entry.id()).unwrap();
        assert!(after_clear.is_empty());
        assert!(!Path::new(inbox.flags_path(entry.id()).as_str()).exists());
    }

    #[test]
    fn delete_message_removes_file_and_flags_meta() {
        let (_dir, client) = client();

        let inbox = client.create_mailbox("inbox").unwrap();
        let entry = client.store(inbox.clone(), b"hello".to_vec()).unwrap();

        let mut flags = crate::flag::M2dirFlags::default();
        flags.insert("$seen");
        client.add_flags(&inbox, entry.id(), flags).unwrap();
        assert!(Path::new(inbox.flags_path(entry.id()).as_str()).exists());

        client.delete_message(inbox.clone(), entry.id()).unwrap();
        assert!(!Path::new(entry.path().as_str()).exists());
        assert!(!Path::new(inbox.flags_path(entry.id()).as_str()).exists());

        let listed = client.list_entries(inbox).unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn delete_mailbox_removes_tree() {
        let (_dir, client) = client();

        let inbox = client.create_mailbox("inbox").unwrap();
        let path = inbox.path().clone();
        assert!(Path::new(path.as_str()).is_dir());

        client.delete_mailbox(path.clone()).unwrap();
        assert!(!Path::new(path.as_str()).exists());
    }

    /// Bring the marker constant into scope for tests.
    use crate::m2store::DOT_M2STORE;
}
