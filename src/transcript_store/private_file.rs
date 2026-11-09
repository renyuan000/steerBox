use super::TranscriptError;
use std::fs::File;
use std::path::Path;

#[cfg(not(unix))]
pub(super) fn create(_: &Path, _: &Path, _: &str) -> Result<File, TranscriptError> {
    Err(TranscriptError::Unsupported)
}

#[cfg(unix)]
pub(super) fn create(
    root: &Path,
    workspace: &Path,
    session: &str,
) -> Result<File, TranscriptError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;

    let (directory, root_chain) = open_directory(root)?;
    let (_, workspace_chain) = open_directory(workspace)?;
    // Inode checks also reject aliases to an overlapping directory. Disallow both
    // ancestor directions, keeping history and source trees structurally separate.
    if root_chain.contains(workspace_chain.last().ok_or(TranscriptError::InvalidRoot)?)
        || workspace_chain.contains(root_chain.last().ok_or(TranscriptError::InvalidRoot)?)
    {
        return Err(TranscriptError::InvalidRoot);
    }
    let metadata = directory.metadata().map_err(io_error)?;
    // SAFETY: geteuid takes no arguments and has no failure mode.
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o7077 != 0 {
        return Err(TranscriptError::InvalidRoot);
    }
    let name =
        CString::new(format!("{session}.jsonl")).map_err(|_| TranscriptError::InvalidEntry)?;
    // SAFETY: directory is an owned open directory, name is NUL-terminated, and
    // O_CREAT supplies its mode argument. O_EXCL never opens an existing inode.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY
                | libc::O_CREAT
                | libc::O_EXCL
                | libc::O_APPEND
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK,
            0o600 as libc::mode_t,
        )
    };
    if fd < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    // SAFETY: successful openat returned a unique owned descriptor.
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata().map_err(io_error)?;
    // SAFETY: geteuid takes no arguments and cannot fail.
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o600
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(TranscriptError::InvalidRoot);
    }
    file.sync_all().map_err(io_error)?;
    directory.sync_all().map_err(io_error)?;
    Ok(file)
}

#[cfg(unix)]
fn io_error(error: std::io::Error) -> TranscriptError {
    super::WriterError::Io(error.kind()).into()
}

#[cfg(unix)]
type DirectoryChain = (File, Vec<(u64, u64)>);

#[cfg(unix)]
fn open_directory(path: &Path) -> Result<DirectoryChain, TranscriptError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    let raw = path.as_os_str().as_bytes();
    if !raw.starts_with(b"/") || raw.contains(&0) {
        return Err(TranscriptError::InvalidRoot);
    }
    let components: Vec<_> = raw[1..].split(|b| *b == b'/').collect();
    if raw != b"/"
        && components
            .iter()
            .any(|c| c.is_empty() || *c == b"." || *c == b"..")
    {
        return Err(TranscriptError::InvalidRoot);
    }
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: static NUL-terminated path and valid open flags; no mode argument needed.
    let fd = unsafe { libc::open(c"/".as_ptr(), flags) };
    if fd < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    // SAFETY: successful open returned a unique owned descriptor.
    let mut directory = unsafe { File::from_raw_fd(fd) };
    let metadata = directory.metadata().map_err(io_error)?;
    let mut chain = vec![(metadata.dev(), metadata.ino())];
    if raw != b"/" {
        for component in components {
            let name = CString::new(component).map_err(|_| TranscriptError::InvalidRoot)?;
            // SAFETY: directory is live and pinned, name has no NUL, flags require
            // a directory and reject symlinks at each individual traversal step.
            let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(io_error(std::io::Error::last_os_error()));
            }
            // SAFETY: successful openat returned a unique owned descriptor.
            directory = unsafe { File::from_raw_fd(fd) };
            let metadata = directory.metadata().map_err(io_error)?;
            chain.push((metadata.dev(), metadata.ino()));
        }
    }
    Ok((directory, chain))
}
