use crate::domain::GoalContract;
use std::fs;
use std::path::{Component, Path};
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
    let target = workspace.join(&relative);
    let before = fs::read_to_string(&target).map_err(io_error)?;
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
    fs::write(&temporary, replacement).map_err(io_error)?;
    if let Err(error) = fs::rename(&temporary, &target) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
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
    let source = workspace.join(&relative);
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
        if !link_path.is_file() {
            missing_links.push(target.to_owned());
        }
    }
    Ok(LinkVerification {
        passed: missing_links.is_empty(),
        checked_links,
        missing_links,
    })
}

fn safe_relative_path(path: &str) -> Option<String> {
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
