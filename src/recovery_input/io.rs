use super::RecoveryInputError;
use std::path::Path;

#[cfg(unix)]
use crate::audit_store::AuditParent;
#[cfg(unix)]
use std::fs::{File, Metadata};
#[cfg(unix)]
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[cfg(unix)]
pub(super) fn read_source(path: &Path, cap: usize) -> Result<Option<Vec<u8>>, RecoveryInputError> {
    if cap == 0 {
        return Err(RecoveryInputError::LimitReached);
    }
    let Some(parent) =
        AuditParent::open(path, false).map_err(|_| RecoveryInputError::UnsafeOrUnavailable)?
    else {
        return Ok(None);
    };
    let Some(mut file) = parent
        .open_read()
        .map_err(|_| RecoveryInputError::UnsafeOrUnavailable)?
    else {
        return Ok(None);
    };
    read_open_file(&mut file, cap, false, |_| {}).map(Some)
}

#[cfg(not(unix))]
pub(super) fn read_source(_: &Path, _: usize) -> Result<Option<Vec<u8>>, RecoveryInputError> {
    Err(RecoveryInputError::UnsafeOrUnavailable)
}

#[cfg(unix)]
pub(super) fn read_private(path: &Path, cap: usize) -> Result<Vec<u8>, RecoveryInputError> {
    let parent = private_parent(path)?;
    let mut file = parent
        .open_read_candidate()
        .map_err(|_| RecoveryInputError::UnsafeOrUnavailable)?
        .ok_or(RecoveryInputError::UnsafeOrUnavailable)?;
    read_open_file(&mut file, cap, true, |_| {})
}

#[cfg(not(unix))]
pub(super) fn read_private(_: &Path, _: usize) -> Result<Vec<u8>, RecoveryInputError> {
    Err(RecoveryInputError::UnsafeOrUnavailable)
}

#[cfg(unix)]
pub(super) fn persist_new(path: &Path, bytes: &[u8]) -> Result<(), RecoveryInputError> {
    persist_new_with_io(
        path,
        bytes,
        |file, bytes| file.write_all(bytes),
        File::sync_all,
        AuditParent::sync_directory,
    )
}

#[cfg(not(unix))]
pub(super) fn persist_new(_: &Path, _: &[u8]) -> Result<(), RecoveryInputError> {
    Err(RecoveryInputError::WriteUnconfirmed)
}

#[cfg(unix)]
fn private_parent(path: &Path) -> Result<AuditParent, RecoveryInputError> {
    let parent = AuditParent::open(path, false)
        .map_err(|_| RecoveryInputError::UnsafeOrUnavailable)?
        .ok_or(RecoveryInputError::UnsafeOrUnavailable)?;
    let metadata = parent
        .directory_metadata()
        .map_err(|_| RecoveryInputError::UnsafeOrUnavailable)?;
    if !metadata.is_dir() {
        return Err(RecoveryInputError::UnsafeOrUnavailable);
    }
    validate_private(&metadata, true)?;
    Ok(parent)
}

#[cfg(unix)]
fn persist_new_with_io(
    path: &Path,
    bytes: &[u8],
    write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    sync_file: impl FnOnce(&File) -> io::Result<()>,
    sync_directory: impl FnOnce(&AuditParent) -> io::Result<()>,
) -> Result<(), RecoveryInputError> {
    let result: Result<(), RecoveryInputError> = (|| {
        let parent = private_parent(path)?;
        let mut file = parent
            .open_create_new_candidate()
            .map_err(|_| RecoveryInputError::WriteUnconfirmed)?;
        let metadata = file
            .metadata()
            .map_err(|_| RecoveryInputError::WriteUnconfirmed)?;
        validate_regular(&metadata)?;
        validate_private(&metadata, false)?;
        write(&mut file, bytes).map_err(|_| RecoveryInputError::WriteUnconfirmed)?;
        sync_file(&file).map_err(|_| RecoveryInputError::WriteUnconfirmed)?;
        sync_directory(&parent).map_err(|_| RecoveryInputError::WriteUnconfirmed)?;
        // The write/sync window must not turn a now-public or linked record into
        // an acknowledged private save. This is not a fence against same-UID writers.
        let metadata = file
            .metadata()
            .map_err(|_| RecoveryInputError::WriteUnconfirmed)?;
        validate_regular(&metadata)?;
        validate_private(&metadata, false)?;
        let metadata = parent
            .directory_metadata()
            .map_err(|_| RecoveryInputError::WriteUnconfirmed)?;
        validate_private(&metadata, true)?;
        Ok(())
    })();
    result.map_err(|_| RecoveryInputError::WriteUnconfirmed)
}

#[cfg(unix)]
fn read_open_file(
    file: &mut File,
    cap: usize,
    private: bool,
    after_read: impl FnOnce(&File),
) -> Result<Vec<u8>, RecoveryInputError> {
    if cap == 0 {
        return Err(RecoveryInputError::LimitReached);
    }
    let before = file
        .metadata()
        .map_err(|_| RecoveryInputError::UnsafeOrUnavailable)?;
    validate_regular(&before)?;
    if private {
        validate_private(&before, false)?;
    }
    let mut bytes = Vec::new();
    let limit = u64::try_from(cap).map_err(|_| RecoveryInputError::LimitReached)?;
    let result = (&mut *file).take(limit).read_to_end(&mut bytes);
    after_read(file);
    let after = file
        .metadata()
        .map_err(|_| RecoveryInputError::UnsafeOrUnavailable)?;
    validate_regular(&after)?;
    if private {
        validate_private(&after, false)?;
    }
    if metadata_changed(&before, &after) || result.is_err() {
        return Err(RecoveryInputError::UnsafeOrUnavailable);
    }
    if bytes.len() == cap {
        return Err(RecoveryInputError::LimitReached);
    }
    Ok(bytes)
}

#[cfg(unix)]
fn validate_regular(metadata: &Metadata) -> Result<(), RecoveryInputError> {
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(RecoveryInputError::UnsafeOrUnavailable);
    }
    Ok(())
}

#[cfg(unix)]
fn validate_private(metadata: &Metadata, directory: bool) -> Result<(), RecoveryInputError> {
    let forbidden_mode = if directory { 0o7077 } else { 0o7177 };
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & forbidden_mode != 0 {
        return Err(RecoveryInputError::UnsafeOrUnavailable);
    }
    Ok(())
}

#[cfg(unix)]
fn metadata_changed(before: &Metadata, after: &Metadata) -> bool {
    let fingerprint = |metadata: &Metadata| {
        (
            metadata.dev(),
            metadata.ino(),
            metadata.mode(),
            metadata.nlink(),
            metadata.len(),
            metadata.uid(),
            metadata.gid(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
    };
    fingerprint(before) != fingerprint(after)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::fs::{self, Permissions};
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

    fn private_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), Permissions::from_mode(0o700)).unwrap();
        root
    }

    #[test]
    fn source_missing_does_not_create_an_entry() {
        let root = private_root();
        let path = root.path().join("missing/source");
        assert_eq!(read_source(&path, 32).unwrap(), None);
        assert!(!root.path().join("missing").exists());
    }

    #[test]
    fn complete_source_read_accepts_public_regular_files() {
        let root = private_root();
        let path = root.path().join("source");
        fs::write(&path, b"sealed\n").unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        assert_eq!(read_source(&path, 32).unwrap(), Some(b"sealed\n".to_vec()));
        assert!(read_private(&path, 32).is_err());
    }

    #[test]
    fn full_cap_is_rejected_without_an_eof_probe() {
        let root = private_root();
        let path = root.path().join("source");
        fs::write(&path, b"abcdef").unwrap();
        assert!(matches!(
            read_source(&path, 6),
            Err(RecoveryInputError::LimitReached)
        ));
        assert!(matches!(
            read_source(&path, 0),
            Err(RecoveryInputError::LimitReached)
        ));
        let parent = AuditParent::open(&path, false).unwrap().unwrap();
        let mut file = parent.open_read().unwrap().unwrap();
        assert!(matches!(
            read_open_file(&mut file, 3, false, |_| {}),
            Err(RecoveryInputError::LimitReached)
        ));
        let mut rest = Vec::new();
        file.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"def");
    }

    #[test]
    fn source_rejects_symlink_parent_leaf_hardlink_and_fifo() {
        let root = private_root();
        let path = root.path().join("source");
        fs::write(&path, b"sealed\n").unwrap();
        let alias = root.path().join("alias");
        symlink(&path, &alias).unwrap();
        assert!(matches!(
            read_source(&alias, 32),
            Err(RecoveryInputError::UnsafeOrUnavailable)
        ));
        let linked_parent = root.path().join("linked");
        symlink(root.path(), &linked_parent).unwrap();
        assert!(read_source(&linked_parent.join("source"), 32).is_err());
        let hardlink = root.path().join("hardlink");
        fs::hard_link(&path, &hardlink).unwrap();
        assert!(read_source(&path, 32).is_err());
        let fifo = root.path().join("fifo");
        let fifo_name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        assert!(read_source(&fifo, 32).is_err());
    }

    #[test]
    fn private_reads_require_private_parent_and_nonexecutable_private_file() {
        let root = private_root();
        let path = root.path().join("record");
        fs::write(&path, b"record").unwrap();
        for mode in [0o600, 0o400] {
            fs::set_permissions(&path, Permissions::from_mode(mode)).unwrap();
            assert_eq!(read_private(&path, 32).unwrap(), b"record");
        }
        for mode in [0o640, 0o604, 0o700, 0o1600, 0o4600] {
            fs::set_permissions(&path, Permissions::from_mode(mode)).unwrap();
            assert!(read_private(&path, 32).is_err(), "mode {mode:o}");
        }
        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(root.path(), Permissions::from_mode(0o750)).unwrap();
        assert!(read_private(&path, 32).is_err());
        assert!(!root.path().join("missing").exists());
        assert!(read_private(&root.path().join("missing"), 32).is_err());
    }

    #[test]
    fn metadata_mutations_reject_the_same_fd_after_read() {
        for mutation in ["contents", "mode", "hardlink"] {
            let root = private_root();
            let path = root.path().join("source");
            fs::write(&path, b"sealed\n").unwrap();
            fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
            let parent = AuditParent::open(&path, false).unwrap().unwrap();
            let mut file = parent.open_read().unwrap().unwrap();
            let result = read_open_file(&mut file, 32, false, |_| match mutation {
                "contents" => fs::write(&path, b"changed content\n").unwrap(),
                "mode" => fs::set_permissions(&path, Permissions::from_mode(0o400)).unwrap(),
                "hardlink" => fs::hard_link(&path, root.path().join("alias")).unwrap(),
                _ => unreachable!(),
            });
            assert!(
                matches!(result, Err(RecoveryInputError::UnsafeOrUnavailable)),
                "{mutation}"
            );
        }
    }

    #[test]
    fn persistence_creates_a_private_record_and_preserves_collisions() {
        let root = private_root();
        let path = root.path().join("record");
        persist_new(&path, b"original").unwrap();
        let before = fs::metadata(&path).unwrap();
        assert_eq!(before.mode() & 0o7777, 0o600);
        assert_eq!(read_private(&path, 32).unwrap(), b"original");
        assert!(matches!(
            persist_new(&path, b"replacement"),
            Err(RecoveryInputError::WriteUnconfirmed)
        ));
        let after = fs::metadata(&path).unwrap();
        assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
        assert_eq!(fs::read(&path).unwrap(), b"original");
    }

    #[test]
    fn persistence_does_not_create_missing_or_public_parents() {
        let root = private_root();
        let missing = root.path().join("missing/record");
        assert!(persist_new(&missing, b"record").is_err());
        assert!(!root.path().join("missing").exists());
        fs::set_permissions(root.path(), Permissions::from_mode(0o755)).unwrap();
        let public = root.path().join("record");
        assert!(persist_new(&public, b"record").is_err());
        assert!(!public.exists());
    }

    #[test]
    fn failed_write_keeps_uncertain_bytes_without_deletion_or_retry() {
        let root = private_root();
        let path = root.path().join("record");
        let result = persist_new_with_io(
            &path,
            b"record",
            |file, _| {
                file.write_all(b"partial")?;
                Err(std::io::Error::other("private error detail"))
            },
            File::sync_all,
            AuditParent::sync_directory,
        );
        assert!(matches!(result, Err(RecoveryInputError::WriteUnconfirmed)));
        assert_eq!(fs::read(&path).unwrap(), b"partial");
        assert!(persist_new(&path, b"retry").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"partial");
    }

    #[test]
    fn changed_privacy_or_link_count_cannot_receive_a_success_ack() {
        for mutation in ["file_mode", "parent_mode", "hardlink"] {
            let root = private_root();
            let path = root.path().join("record");
            let result = persist_new_with_io(
                &path,
                b"complete",
                |file, bytes| {
                    file.write_all(bytes)?;
                    match mutation {
                        "file_mode" => file.set_permissions(Permissions::from_mode(0o644)),
                        "parent_mode" => {
                            fs::set_permissions(root.path(), Permissions::from_mode(0o755))
                        }
                        "hardlink" => fs::hard_link(&path, root.path().join("alias")),
                        _ => unreachable!(),
                    }
                },
                File::sync_all,
                AuditParent::sync_directory,
            );
            assert!(
                matches!(result, Err(RecoveryInputError::WriteUnconfirmed)),
                "{mutation}"
            );
            assert_eq!(fs::read(&path).unwrap(), b"complete");
        }
    }

    #[test]
    fn sync_failures_keep_complete_uncertain_records_without_retry() {
        for failure in ["file", "directory"] {
            let root = private_root();
            let path = root.path().join("record");
            let result = persist_new_with_io(
                &path,
                b"complete",
                |file, bytes| file.write_all(bytes),
                |file| {
                    if failure == "file" {
                        Err(std::io::Error::other("private file sync detail"))
                    } else {
                        file.sync_all()
                    }
                },
                |_| Err(std::io::Error::other("private directory sync detail")),
            );
            assert!(matches!(result, Err(RecoveryInputError::WriteUnconfirmed)));
            assert_eq!(fs::read(&path).unwrap(), b"complete");
            let before = fs::metadata(&path).unwrap();
            assert!(persist_new(&path, b"retry").is_err());
            let after = fs::metadata(&path).unwrap();
            assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
            assert_eq!(fs::read(&path).unwrap(), b"complete");
        }
    }

    #[test]
    fn directory_replacement_does_not_redirect_the_pinned_save() {
        let root = private_root();
        let parent = root.path().join("private");
        let moved = root.path().join("moved");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, Permissions::from_mode(0o700)).unwrap();
        let path = parent.join("record");
        persist_new_with_io(
            &path,
            b"complete",
            |file, bytes| {
                fs::rename(&parent, &moved)?;
                fs::create_dir(&parent)?;
                fs::set_permissions(&parent, Permissions::from_mode(0o700))?;
                file.write_all(bytes)
            },
            File::sync_all,
            AuditParent::sync_directory,
        )
        .unwrap();
        assert_eq!(fs::read(moved.join("record")).unwrap(), b"complete");
        assert!(!path.exists());
    }
}
