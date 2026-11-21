use super::AuditStoreError;
use std::fs::File;
use std::io;
use std::path::Path;

#[cfg(unix)]
use std::ffi::{CStr, CString};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
#[cfg(unix)]
use std::path::Component;

pub(super) struct AuditParent {
    #[cfg(unix)]
    directory: File,
    #[cfg(unix)]
    name: CString,
}

#[cfg(unix)]
struct ParsedPath {
    absolute: bool,
    parents: Vec<CString>,
    name: CString,
}

#[cfg(unix)]
fn parse_path(path: &Path) -> Result<ParsedPath, AuditStoreError> {
    let bytes = path.as_os_str().as_bytes();
    let basename = bytes
        .rsplit(|byte| *byte == b'/')
        .next()
        .unwrap_or_default();
    if bytes.contains(&0) || basename.is_empty() || basename == b"." || basename == b".." {
        return Err(io_error(
            "path validation",
            io::ErrorKind::InvalidInput.into(),
        ));
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => components
                .push(CString::new(name.as_bytes()).map_err(|_| {
                    io_error("path validation", io::ErrorKind::InvalidInput.into())
                })?),
            _ => {
                return Err(io_error(
                    "path validation",
                    io::ErrorKind::InvalidInput.into(),
                ))
            }
        }
    }
    let name = components
        .pop()
        .ok_or_else(|| io_error("path validation", io::ErrorKind::InvalidInput.into()))?;
    Ok(ParsedPath {
        absolute: path.is_absolute(),
        parents: components,
        name,
    })
}

#[cfg(unix)]
impl AuditParent {
    pub(super) fn open(path: &Path, create: bool) -> Result<Option<Self>, AuditStoreError> {
        Self::open_with_sync(path, create, File::sync_all)
    }

    fn open_with_sync(
        path: &Path,
        create: bool,
        mut sync: impl FnMut(&File) -> io::Result<()>,
    ) -> Result<Option<Self>, AuditStoreError> {
        let parsed = parse_path(path)?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let anchor = if parsed.absolute { c"/" } else { c"." };
        // SAFETY: anchor is a static NUL-terminated CStr. These directory-open
        // flags omit O_CREAT, so open requires no variadic mode argument.
        let descriptor = unsafe { libc::open(anchor.as_ptr(), flags) };
        if descriptor < 0 {
            return Err(io_error("parent open", io::Error::last_os_error()));
        }
        // SAFETY: successful open returned a new, uniquely owned descriptor;
        // File takes ownership and closes it on replacement or any return path.
        let mut directory = unsafe { File::from_raw_fd(descriptor) };
        for name in parsed.parents {
            let child = match open_at(&directory, &name, flags, 0) {
                Ok(child) => child,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if !create {
                        return Ok(None);
                    }
                    // SAFETY: directory owns a live directory descriptor, name
                    // is a validated single-component CString, and 0700 is a valid mode.
                    let result =
                        unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
                    if result < 0 {
                        let error = io::Error::last_os_error();
                        if error.kind() != io::ErrorKind::AlreadyExists {
                            return Err(io_error("parent create", error));
                        }
                    }
                    open_at(&directory, &name, flags, 0)
                        .map_err(|error| io_error("parent open", error))?
                }
                Err(error) => return Err(io_error("parent open", error)),
            };
            if create {
                sync(&directory).map_err(|error| io_error("parent directory sync", error))?;
            }
            directory = child;
        }
        if create {
            sync(&directory).map_err(|error| io_error("parent directory sync", error))?;
        }
        Ok(Some(Self {
            directory,
            name: parsed.name,
        }))
    }

    pub(super) fn open_append(&self) -> Result<File, AuditStoreError> {
        let flags = libc::O_WRONLY
            | libc::O_CREAT
            | libc::O_APPEND
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | libc::O_CLOEXEC;
        let file = open_at(&self.directory, &self.name, flags, 0o600)
            .map_err(|error| io_error("file open", error))?;
        validate_file(&file)?;
        Ok(file)
    }

    pub(super) fn open_read(&self) -> Result<Option<File>, AuditStoreError> {
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
        let file = match open_at(&self.directory, &self.name, flags, 0) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error("file open", error)),
        };
        validate_file(&file)?;
        Ok(Some(file))
    }

    pub(super) fn sync_directory(&self) -> io::Result<()> {
        self.directory.sync_all()
    }
}

#[cfg(unix)]
fn open_at(
    directory: &File,
    name: &CStr,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> io::Result<File> {
    // SAFETY: directory owns a live directory descriptor and name is a valid
    // NUL-terminated, single-component CStr. The mode argument is supplied for
    // O_CREAT callers and ignored by openat when creation is not requested.
    let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, mode) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openat returned a new, uniquely owned descriptor;
    // ownership transfers immediately to File, whose Drop closes it on errors.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn validate_file(file: &File) -> Result<(), AuditStoreError> {
    let metadata = file
        .metadata()
        .map_err(|error| io_error("file metadata", error))?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(io_error(
            "file validation",
            io::ErrorKind::InvalidInput.into(),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
impl AuditParent {
    pub(super) fn open(_: &Path, _: bool) -> Result<Option<Self>, AuditStoreError> {
        Err(io_error("parent open", io::ErrorKind::Unsupported.into()))
    }

    pub(super) fn open_append(&self) -> Result<File, AuditStoreError> {
        Err(io_error("file open", io::ErrorKind::Unsupported.into()))
    }

    pub(super) fn open_read(&self) -> Result<Option<File>, AuditStoreError> {
        Err(io_error("file open", io::ErrorKind::Unsupported.into()))
    }

    pub(super) fn sync_directory(&self) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

fn io_error(stage: &str, error: io::Error) -> AuditStoreError {
    AuditStoreError::Io {
        message: format!("audit {stage} failed: {:?}", error.kind()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn pinned_parent_rename_does_not_redirect_leaf_access() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original");
        fs::create_dir(&original).unwrap();
        let parent = AuditParent::open(&original.join("log.jsonl"), true)
            .unwrap()
            .unwrap();
        let moved = root.path().join("moved");
        fs::rename(&original, &moved).unwrap();
        fs::create_dir(&original).unwrap();
        let mut file = parent.open_append().unwrap();
        file.write_all(b"{}\n").unwrap();
        file.sync_data().unwrap();
        parent.sync_directory().unwrap();
        assert_eq!(fs::read(moved.join("log.jsonl")).unwrap(), b"{}\n");
        assert!(!original.join("log.jsonl").exists());
        assert_eq!(
            parent
                .open_read()
                .unwrap()
                .unwrap()
                .metadata()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn invalid_paths_fail_before_any_directory_creation() {
        let root = tempfile::tempdir().unwrap();
        for suffix in ["new/../log", "new/.", "new/", "new/.."] {
            assert!(AuditParent::open(&root.path().join(suffix), true).is_err());
            assert!(!root.path().join("new").exists());
        }
        for bytes in [b"".as_slice(), b"/", b".", b"..", b"./", b"bad\0name"] {
            assert!(
                AuditParent::open(Path::new(std::ffi::OsStr::from_bytes(bytes)), true).is_err()
            );
        }
    }

    #[test]
    fn leading_dot_and_non_utf8_names_are_preserved() {
        let parsed = parse_path(Path::new("./nested//log.jsonl")).unwrap();
        assert!(!parsed.absolute);
        assert_eq!(parsed.parents.len(), 1);
        assert_eq!(parsed.parents[0].as_bytes(), b"nested");
        assert_eq!(parsed.name.as_bytes(), b"log.jsonl");
        let root = tempfile::tempdir().unwrap();
        let path = root
            .path()
            .join(std::ffi::OsStr::from_bytes(b"log-\xff.jsonl"));
        let parent = AuditParent::open(&path, true).unwrap().unwrap();
        parent.open_append().unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn missing_read_does_not_create_parent_or_file() {
        let root = tempfile::tempdir().unwrap();
        assert!(AuditParent::open(&root.path().join("missing/log"), false)
            .unwrap()
            .is_none());
        assert!(!root.path().join("missing").exists());
        let parent = AuditParent::open(&root.path().join("log"), false)
            .unwrap()
            .unwrap();
        assert!(parent.open_read().unwrap().is_none());
        assert!(!root.path().join("log").exists());
    }

    #[test]
    fn nested_creation_syncs_each_directory_and_retry_repeats_failed_barrier() {
        let root = tempfile::tempdir().unwrap();
        let root_metadata = root.path().metadata().unwrap();
        let root_identity = (root_metadata.dev(), root_metadata.ino());
        let path = root.path().join("new/nested/log");
        let result = AuditParent::open_with_sync(&path, true, |directory| {
            let metadata = directory.metadata()?;
            if (metadata.dev(), metadata.ino()) == root_identity {
                return Err(io::Error::other("synthetic directory sync detail"));
            }
            Ok(())
        });
        let error = result.err().expect("parent sync failure is propagated");
        assert!(!error
            .to_string()
            .contains("synthetic directory sync detail"));
        assert!(root.path().join("new").is_dir());
        assert!(!root.path().join("new/nested").exists());
        let mut synced = Vec::new();
        let parent = AuditParent::open_with_sync(&path, true, |directory| {
            let metadata = directory.metadata()?;
            synced.push((metadata.dev(), metadata.ino()));
            directory.sync_all()
        })
        .unwrap()
        .unwrap();
        for directory in [
            root.path().to_path_buf(),
            root.path().join("new"),
            root.path().join("new/nested"),
        ] {
            let metadata = directory.metadata().unwrap();
            assert!(synced.contains(&(metadata.dev(), metadata.ino())));
        }
        assert_eq!(
            root.path().join("new").metadata().unwrap().mode() & 0o077,
            0
        );
        parent.open_append().unwrap();
        parent.sync_directory().unwrap();
    }
}
