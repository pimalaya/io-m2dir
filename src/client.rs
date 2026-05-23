//! Standard, blocking m2dir client.
//!
//! Holds a single filesystem root [`PathBuf`] and exposes one method
//! per coroutine. Every method runs its coroutine to completion by
//! performing the requested filesystem operations via [`std::fs`] in a
//! resume loop.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use log::trace;
use thiserror::Error;

use crate::coroutines::{
    flags_add::*, flags_remove::*, flags_set::*, mailbox_create::*, mailbox_delete::*,
    mailbox_list::*, message_delete::*, message_get::*, message_list::*, message_store::*,
};
use crate::entry::Entry;
use crate::flag::Flags;
use crate::m2dir::{LoadM2dirError, M2dir};
use crate::m2store::{LoadM2storeError, M2store, NewFolderError};

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
    MailboxCreate(#[from] MailboxCreateError),

    #[error(transparent)]
    MailboxDelete(#[from] MailboxDeleteError),

    #[error(transparent)]
    MailboxList(#[from] MailboxListError),

    #[error(transparent)]
    MessageList(#[from] MessageListError),

    #[error(transparent)]
    MessageGet(#[from] MessageGetError),

    #[error(transparent)]
    MessageStore(#[from] MessageStoreError),

    #[error(transparent)]
    MessageDelete(#[from] MessageDeleteError),

    #[error(transparent)]
    FlagsAdd(#[from] FlagsAddError),

    #[error(transparent)]
    FlagsRemove(#[from] FlagsRemoveError),

    #[error(transparent)]
    FlagsSet(#[from] FlagsSetError),

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
    root: PathBuf,
}

impl M2dirClient {
    /// Builds a client rooted at `root`. No filesystem check is
    /// performed at construction time.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the filesystem root this client operates on.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Opens the m2store at the client root, returning a typed handle
    /// on success.
    pub fn open_store(&self) -> Result<M2store, M2dirClientError> {
        Ok(M2store::try_from(self.root.clone())?)
    }

    /// Initialises a brand new m2store at the client root: creates
    /// the directory if needed and writes the `.m2store` marker.
    pub fn init_store(&self) -> Result<M2store, M2dirClientError> {
        trace!("init m2store at {}", self.root.display());

        fs::create_dir_all(&self.root)?;
        let marker = self.root.join(crate::m2store::DOT_M2STORE);
        if !marker.exists() {
            fs::write(&marker, b"")?;
        }

        Ok(M2store::from_path(self.root.clone()))
    }

    // ---- Mailbox lifecycle ----------------------------------------------

    /// Runs [`MailboxCreate`]: creates the m2dir folder `name` and
    /// writes the `.m2dir` marker.
    pub fn create_mailbox(&self, name: impl AsRef<str>) -> Result<M2dir, M2dirClientError> {
        let store = self.open_store()?;
        let mut coroutine = MailboxCreate::new(&store, name)?;
        let mut arg: Option<MailboxCreateArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                MailboxCreateResult::Ok(m2dir) => return Ok(m2dir),
                MailboxCreateResult::WantsDirCreate(paths) => {
                    create_dirs(paths)?;
                    arg = Some(MailboxCreateArg::DirCreate);
                }
                MailboxCreateResult::WantsFileCreate(files) => {
                    write_files(files)?;
                    arg = Some(MailboxCreateArg::FileCreate);
                }
                MailboxCreateResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Runs [`MailboxDelete`]: recursively removes the m2dir at
    /// `path`.
    pub fn delete_mailbox(&self, path: impl AsRef<Path>) -> Result<(), M2dirClientError> {
        let mut coroutine = MailboxDelete::new(path);
        let mut arg: Option<MailboxDeleteArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                MailboxDeleteResult::Ok => return Ok(()),
                MailboxDeleteResult::WantsDirRemove(paths) => {
                    remove_dirs(paths)?;
                    arg = Some(MailboxDeleteArg::DirRemove);
                }
                MailboxDeleteResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Runs [`MailboxList`]: returns every m2dir found under the
    /// store root.
    pub fn list_mailboxes(&self) -> Result<HashSet<M2dir>, M2dirClientError> {
        let store = self.open_store()?;
        let mut coroutine = MailboxList::new(&store);
        let mut arg: Option<MailboxListArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                MailboxListResult::Ok(found) => return Ok(found),
                MailboxListResult::WantsDirRead(paths) => {
                    let entries = read_dirs(paths)?;
                    arg = Some(MailboxListArg::DirRead(entries));
                }
                MailboxListResult::Err(err) => return Err(err.into()),
            }
        }
    }

    // ---- Messages -------------------------------------------------------

    /// Runs [`MessageList`]: returns every entry inside `m2dir`.
    pub fn list_messages(&self, m2dir: M2dir) -> Result<Vec<Entry>, M2dirClientError> {
        let mut coroutine = MessageList::new(m2dir);
        let mut arg: Option<MessageListArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                MessageListResult::Ok(entries) => return Ok(entries),
                MessageListResult::WantsDirRead(paths) => {
                    let entries = read_dirs(paths)?;
                    arg = Some(MessageListArg::DirRead(entries));
                }
                MessageListResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Runs [`MessageGet`]: locates and reads entry `id` from
    /// `m2dir`, validating the checksum embedded in the filename.
    pub fn get(
        &self,
        m2dir: M2dir,
        id: impl ToString,
    ) -> Result<(Entry, Vec<u8>), M2dirClientError> {
        let mut coroutine = MessageGet::new(m2dir, id);
        let mut arg: Option<MessageGetArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                MessageGetResult::Ok { entry, contents } => return Ok((entry, contents)),
                MessageGetResult::WantsDirRead(paths) => {
                    let entries = read_dirs(paths)?;
                    arg = Some(MessageGetArg::DirRead(entries));
                }
                MessageGetResult::WantsFileRead(paths) => {
                    let files = read_files(paths)?;
                    arg = Some(MessageGetArg::FileRead(files));
                }
                MessageGetResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Runs [`MessageStore`]: writes `bytes` to a temporary file
    /// inside `m2dir`, then atomically renames it to its checksum-
    /// based final filename.
    pub fn store(&self, m2dir: &M2dir, bytes: Vec<u8>) -> Result<Entry, M2dirClientError> {
        let mut coroutine = MessageStore::new(m2dir, bytes);
        let mut arg: Option<MessageStoreArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                MessageStoreResult::Ok(entry) => return Ok(entry),
                MessageStoreResult::WantsFileCreate(files) => {
                    write_files(files)?;
                    arg = Some(MessageStoreArg::FileCreate);
                }
                MessageStoreResult::WantsRename(pairs) => {
                    rename_paths(pairs)?;
                    arg = Some(MessageStoreArg::Rename);
                }
                MessageStoreResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Runs [`MessageDelete`]: removes entry `id` and every matching
    /// `.meta/<id>*` sidecar.
    pub fn delete_message(
        &self,
        m2dir: M2dir,
        id: impl ToString,
    ) -> Result<(), M2dirClientError> {
        let mut coroutine = MessageDelete::new(m2dir, id);
        let mut arg: Option<MessageDeleteArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                MessageDeleteResult::Ok => return Ok(()),
                MessageDeleteResult::WantsDirRead(paths) => {
                    let entries = read_dirs(paths)?;
                    arg = Some(MessageDeleteArg::DirRead(entries));
                }
                MessageDeleteResult::WantsFileRemove(paths) => {
                    remove_files(paths)?;
                    arg = Some(MessageDeleteArg::FileRemove);
                }
                MessageDeleteResult::Err(err) => return Err(err.into()),
            }
        }
    }

    // ---- Flags ----------------------------------------------------------

    /// Reads the `.flags` sidecar for entry `id` inside `m2dir`,
    /// returning an empty set if the file is missing.
    pub fn read_flags(
        &self,
        m2dir: &M2dir,
        id: impl AsRef<str>,
    ) -> Result<Flags, M2dirClientError> {
        let path = m2dir.flags_sidecar_path(id.as_ref());
        trace!("read flags sidecar at {}", path.display());

        match fs::read_to_string(&path) {
            Ok(text) => Ok(Flags::from_sidecar(&text)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Flags::default()),
            Err(err) => Err(err.into()),
        }
    }

    /// Runs [`FlagsAdd`]: adds `flags` to entry `id`'s sidecar.
    pub fn add_flags(
        &self,
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: Flags,
    ) -> Result<(), M2dirClientError> {
        let mut coroutine = FlagsAdd::new(m2dir, id, flags);
        let mut arg: Option<FlagsAddArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                FlagsAddResult::Ok => return Ok(()),
                FlagsAddResult::WantsFileRead(paths) => {
                    let files = read_files_tolerant(paths)?;
                    arg = Some(FlagsAddArg::FileRead(files));
                }
                FlagsAddResult::WantsFileCreate(files) => {
                    write_files(files)?;
                    arg = Some(FlagsAddArg::FileCreate);
                }
                FlagsAddResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Runs [`FlagsRemove`]: removes `flags` from entry `id`'s
    /// sidecar. When the resulting set is empty the sidecar file is
    /// deleted.
    pub fn remove_flags(
        &self,
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: Flags,
    ) -> Result<(), M2dirClientError> {
        let mut coroutine = FlagsRemove::new(m2dir, id, flags);
        let mut arg: Option<FlagsRemoveArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                FlagsRemoveResult::Ok => return Ok(()),
                FlagsRemoveResult::WantsFileRead(paths) => {
                    let files = read_files_tolerant(paths)?;
                    arg = Some(FlagsRemoveArg::FileRead(files));
                }
                FlagsRemoveResult::WantsFileCreate(files) => {
                    write_files(files)?;
                    arg = Some(FlagsRemoveArg::FileCreate);
                }
                FlagsRemoveResult::WantsFileRemove(paths) => {
                    remove_files(paths)?;
                    arg = Some(FlagsRemoveArg::FileRemove);
                }
                FlagsRemoveResult::Err(err) => return Err(err.into()),
            }
        }
    }

    /// Runs [`FlagsSet`]: replaces entry `id`'s sidecar with `flags`,
    /// deleting it when `flags` is empty.
    pub fn set_flags(
        &self,
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: Flags,
    ) -> Result<(), M2dirClientError> {
        let mut coroutine = FlagsSet::new(m2dir, id, flags);
        let mut arg: Option<FlagsSetArg> = None;

        loop {
            match coroutine.resume(arg.take()) {
                FlagsSetResult::Ok => return Ok(()),
                FlagsSetResult::WantsFileCreate(files) => {
                    write_files(files)?;
                    arg = Some(FlagsSetArg::FileCreate);
                }
                FlagsSetResult::WantsFileRemove(paths) => {
                    remove_files_tolerant(paths)?;
                    arg = Some(FlagsSetArg::FileRemove);
                }
                FlagsSetResult::Err(err) => return Err(err.into()),
            }
        }
    }
}

fn create_dirs(paths: BTreeSet<String>) -> Result<(), io::Error> {
    for path in paths {
        trace!("create_dir_all {path}");
        fs::create_dir_all(&path)?;
    }
    Ok(())
}

fn remove_dirs(paths: BTreeSet<String>) -> Result<(), io::Error> {
    for path in paths {
        trace!("remove_dir_all {path}");
        fs::remove_dir_all(&path)?;
    }
    Ok(())
}

fn write_files(files: BTreeMap<String, Vec<u8>>) -> Result<(), io::Error> {
    for (path, contents) in files {
        trace!("write {path} ({} bytes)", contents.len());

        if let Some(parent) = Path::new(&path).parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &contents)?;
    }
    Ok(())
}

fn remove_files(paths: BTreeSet<String>) -> Result<(), io::Error> {
    for path in paths {
        trace!("remove_file {path}");
        fs::remove_file(&path)?;
    }
    Ok(())
}

fn remove_files_tolerant(paths: BTreeSet<String>) -> Result<(), io::Error> {
    for path in paths {
        trace!("remove_file (tolerant) {path}");
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn read_dirs(paths: BTreeSet<String>) -> Result<BTreeMap<String, BTreeSet<String>>, io::Error> {
    let mut entries = BTreeMap::new();

    for path in paths {
        trace!("read_dir {path}");

        let mut names = BTreeSet::new();
        match fs::read_dir(&path) {
            Ok(iter) => {
                for entry in iter {
                    let entry = entry?;
                    names.insert(entry.path().to_string_lossy().into_owned());
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }

        entries.insert(path, names);
    }

    Ok(entries)
}

fn read_files(paths: BTreeSet<String>) -> Result<BTreeMap<String, Vec<u8>>, io::Error> {
    let mut contents = BTreeMap::new();

    for path in paths {
        trace!("read_file {path}");
        let bytes = fs::read(&path)?;
        contents.insert(path, bytes);
    }

    Ok(contents)
}

fn read_files_tolerant(paths: BTreeSet<String>) -> Result<BTreeMap<String, Vec<u8>>, io::Error> {
    let mut contents = BTreeMap::new();

    for path in paths {
        trace!("read_file (tolerant) {path}");
        match fs::read(&path) {
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

fn rename_paths(pairs: Vec<(String, String)>) -> Result<(), io::Error> {
    for (from, to) in pairs {
        trace!("rename {from} -> {to}");
        fs::rename(&from, &to)?;
    }
    Ok(())
}
