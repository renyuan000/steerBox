//! Complete bounded reads for local verification, not display/prompt content.

use crate::audit_store::AuditParent;
use std::{
    io::{self, Read},
    path::Path,
};

pub(crate) const MAX_WORKSPACE_VERIFICATION_BYTES: usize = 2 * 1024 * 1024;

/// The caller owns path policy. Pin the parent and validate the actual regular
/// file FD; allow source hardlinks, but never follow a symlink during open.
/// This does not freeze bytes against concurrent writers or reserve run identity.
pub(crate) fn read_regular_bounded(path: &Path) -> io::Result<Vec<u8>> {
    let parent =
        AuditParent::open(path, false)?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    let file = parent.open_read_for_replacement()?;
    let mut bytes = Vec::new();
    file.take((MAX_WORKSPACE_VERIFICATION_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_WORKSPACE_VERIFICATION_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}
