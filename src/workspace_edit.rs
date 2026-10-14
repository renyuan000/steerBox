use crate::domain::GoalContract;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkspaceEditError {
    #[error("file write requires explicit human approval")]
    ApprovalRequired,
    #[error("path is outside goal scope: {0}")]
    OutOfScope(String),
    #[error("only Markdown files may be edited: {0}")]
    NotMarkdown(String),
    #[error("workspace file is invalid: {0}")]
    InvalidPath(String),
    #[error("workspace edit I/O failed: {message}")]
    Io { message: String },
    #[error("expected file content did not match")]
    ExpectedContentMismatch,
    #[error("workspace path crosses a symbolic link: {0}")]
    SymbolicLink(String),
    #[error("workspace path resolves outside workspace: {0}")]
    OutsideWorkspaceResolved(String),
    #[error("workspace path is not a regular file: {0}")]
    NotRegularFile(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditResult {
    pub relative_path: String,
    pub changed: bool,
    pub bytes_before: usize,
    pub bytes_after: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkVerification {
    pub passed: bool,
    pub checked_links: Vec<String>,
    pub missing_links: Vec<String>,
}

pub fn edit_markdown(
    workspace: &Path,
    goal: &GoalContract,
    relative_path: &str,
    expected: &str,
    replacement: &str,
    approved: bool,
) -> Result<EditResult, WorkspaceEditError> {
    if !approved {
        return Err(WorkspaceEditError::ApprovalRequired);
    }
    let relative = safe_relative_path(relative_path)
        .ok_or_else(|| WorkspaceEditError::InvalidPath(relative_path.into()))?;
    if !goal.is_within_scope(&relative) {
        return Err(WorkspaceEditError::OutOfScope(relative));
    }
    if !relative.ends_with(".md") && !relative.ends_with(".markdown") {
        return Err(WorkspaceEditError::NotMarkdown(relative));
    }
    let target = checked_existing_file(workspace, &relative)?;
    let mut before = String::new();
    fs::File::open(&target)
        .map_err(io_error)?
        .take(expected.len() as u64 + 1)
        .read_to_string(&mut before)
        .map_err(io_error)?;
    if before != expected {
        return Err(WorkspaceEditError::ExpectedContentMismatch);
    }
    if before == replacement {
        return Ok(EditResult {
            relative_path: relative,
            changed: false,
            bytes_before: before.len(),
            bytes_after: before.len(),
        });
    }
    let temporary = target.with_extension(format!(
        "{}.steerbox-tmp-{}",
        target
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("md"),
        std::process::id()
    ));
    let permissions = fs::metadata(&target).map_err(io_error)?.permissions();
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut temporary_file = options.open(&temporary).map_err(io_error)?;
    let staged = (|| {
        temporary_file.write_all(replacement.as_bytes())?;
        temporary_file.set_permissions(permissions)?;
        temporary_file.sync_all()
    })();
    drop(temporary_file);
    if let Err(error) = staged {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    if let Err(error) = fs::rename(&temporary, &target) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    #[cfg(unix)]
    fs::File::open(
        target
            .parent()
            .ok_or_else(|| WorkspaceEditError::InvalidPath(relative.clone()))?,
    )
    .and_then(|directory| directory.sync_all())
    .map_err(io_error)?;
    Ok(EditResult {
        relative_path: relative,
        changed: true,
        bytes_before: before.len(),
        bytes_after: replacement.len(),
    })
}

pub fn verify_markdown_links(
    workspace: &Path,
    relative_path: &str,
) -> Result<LinkVerification, WorkspaceEditError> {
    let relative = safe_relative_path(relative_path)
        .ok_or_else(|| WorkspaceEditError::InvalidPath(relative_path.into()))?;
    if !relative.ends_with(".md") && !relative.ends_with(".markdown") {
        return Err(WorkspaceEditError::NotMarkdown(relative));
    }
    let source = checked_existing_file(workspace, &relative)?;
    let contents = fs::read_to_string(&source).map_err(io_error)?;
    let mut checked_links = Vec::new();
    let mut missing_links = Vec::new();
    let mut remaining = contents.as_str();
    while let Some(start) = remaining.find("](") {
        let target_start = start + 2;
        let Some(end) = remaining[target_start..].find(')') else {
            break;
        };
        let target = &remaining[target_start..target_start + end];
        remaining = &remaining[target_start + end + 1..];
        if target.is_empty() || target.starts_with('#') || target.contains("://") {
            continue;
        }
        let target = target.split('#').next().unwrap_or(target);
        checked_links.push(target.to_owned());
        let link_path = source.parent().unwrap_or(workspace).join(target);
        if !resolved_regular_file_within_workspace(workspace, &link_path) {
            missing_links.push(target.to_owned());
        }
    }
    Ok(LinkVerification {
        passed: missing_links.is_empty(),
        checked_links,
        missing_links,
    })
}

pub(crate) fn checked_existing_file(
    workspace: &Path,
    relative: &str,
) -> Result<PathBuf, WorkspaceEditError> {
    let target = workspace.join(relative);
    let metadata = fs::symlink_metadata(&target).map_err(io_error)?;
    if metadata.file_type().is_symlink() {
        return Err(WorkspaceEditError::SymbolicLink(relative.into()));
    }
    if !metadata.is_file() {
        return Err(WorkspaceEditError::NotRegularFile(relative.into()));
    }
    ensure_no_symlink_ancestors(workspace, &target, relative)?;
    let workspace_canonical = fs::canonicalize(workspace).map_err(io_error)?;
    let target_canonical = fs::canonicalize(&target).map_err(io_error)?;
    if !target_canonical.starts_with(&workspace_canonical) {
        return Err(WorkspaceEditError::OutsideWorkspaceResolved(
            relative.into(),
        ));
    }
    Ok(target)
}

fn ensure_no_symlink_ancestors(
    workspace: &Path,
    target: &Path,
    relative: &str,
) -> Result<(), WorkspaceEditError> {
    let mut current = target.parent();
    while let Some(path) = current {
        if path == workspace {
            break;
        }
        let metadata = fs::symlink_metadata(path).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(WorkspaceEditError::SymbolicLink(relative.into()));
        }
        current = path.parent();
    }
    Ok(())
}

fn resolved_regular_file_within_workspace(workspace: &Path, path: &Path) -> bool {
    let Ok(workspace_canonical) = fs::canonicalize(workspace) else {
        return false;
    };
    let Ok(path_metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !path_metadata.is_file() {
        return false;
    }
    let Ok(path_canonical) = fs::canonicalize(path) else {
        return false;
    };
    path_canonical.starts_with(workspace_canonical)
}

pub(crate) fn safe_relative_path(path: &str) -> Option<String> {
    if path.is_empty() || path.contains('\\') || Path::new(path).is_absolute() {
        return None;
    }
    let mut components = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(value) => components.push(value.to_str()?.to_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if components.is_empty() {
        None
    } else {
        Some(components.join("/"))
    }
}

fn io_error(error: std::io::Error) -> WorkspaceEditError {
    WorkspaceEditError::Io {
        message: error.to_string(),
    }
}
