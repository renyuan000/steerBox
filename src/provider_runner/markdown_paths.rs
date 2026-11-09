use super::*;
use std::path::{Component, Path};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct MarkdownWorkspaceIdentity {
    canonical: PathBuf,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

/// Metadata-only frozen-scope check. This is not a write permit, content
/// snapshot, history check or protection from concurrent OS-level replacement.
pub struct MarkdownRuntimePathCheck {
    run: BoundedMarkdownRunConfig,
    identity: MarkdownWorkspaceIdentity,
    events: PathBuf,
    model_calls: PathBuf,
    projection: Option<PathBuf>,
}

impl MarkdownRuntimePathCheck {
    pub fn revalidate(&self) -> Result<(), ProviderRunError> {
        if validate_scope(&self.run)? != self.identity {
            return Err(ProviderRunError::InvalidRequest);
        }
        let tools = tool_call_store_path(&self.events);
        let mut tool_lock = tools.as_os_str().to_os_string();
        tool_lock.push(".lock");
        let mut paths = vec![
            self.events.clone(),
            self.run.checkpoint_path.clone(),
            self.run.trace_path.clone(),
            self.run.side_effect_path.clone(),
            tools,
            PathBuf::from(tool_lock),
        ];
        paths.extend(ModelCallStore::storage_paths_for(&self.model_calls));
        paths.extend(self.projection.iter().cloned());
        let mut keys = Vec::with_capacity(paths.len());
        for path in &paths {
            if path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
            {
                return Err(ProviderRunError::InvalidRequest);
            }
            validate_storage_path(path)?;
            let key = normalized_path_key(path)?;
            if keys.contains(&key) {
                return Err(ProviderRunError::InvalidRequest);
            }
            keys.push(key);
        }
        for allowed in &self.run.allowed_paths {
            let target = self.run.workspace.join(allowed);
            if keys.contains(&normalized_path_key(&target)?) {
                return Err(ProviderRunError::InvalidRequest);
            }
            paths.push(target);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let mut identities = Vec::with_capacity(paths.len());
            for path in paths {
                match fs::symlink_metadata(path) {
                    Ok(metadata) => {
                        let id = (metadata.dev(), metadata.ino());
                        if !metadata.is_file() || metadata.nlink() != 1 || identities.contains(&id)
                        {
                            return Err(ProviderRunError::InvalidRequest);
                        }
                        identities.push(id);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(_) => return Err(ProviderRunError::InvalidRequest),
                }
            }
        }
        Ok(())
    }
}

impl BoundedMarkdownProviderRun {
    /// Check every runtime destination before the caller opens any store.
    /// No directories/files, credential resolution or network access occur.
    pub fn validate_runtime_paths(
        &self,
        events: &Path,
        model_calls: &Path,
        projection: Option<&Path>,
    ) -> Result<MarkdownRuntimePathCheck, ProviderRunError> {
        let check = MarkdownRuntimePathCheck {
            run: self.run.clone(),
            identity: self.workspace_identity.clone(),
            events: events.to_owned(),
            model_calls: model_calls.to_owned(),
            projection: projection.map(Path::to_owned),
        };
        check.revalidate()?;
        Ok(check)
    }
}

#[cfg(not(unix))]
pub(super) fn validate_scope(
    _: &BoundedMarkdownRunConfig,
) -> Result<MarkdownWorkspaceIdentity, ProviderRunError> {
    Err(ProviderRunError::InvalidRequest)
}

#[cfg(unix)]
pub(super) fn validate_scope(
    run: &BoundedMarkdownRunConfig,
) -> Result<MarkdownWorkspaceIdentity, ProviderRunError> {
    use std::os::unix::fs::MetadataExt;
    if run.workspace.to_str().is_none()
        || !run.workspace.is_absolute()
        || run.workspace.parent().is_none()
        || run
            .workspace
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        || run.allowed_paths.is_empty()
        || run.allowed_paths.len() > 256
    {
        return Err(ProviderRunError::InvalidRequest);
    }
    reject_symlink_components(&run.workspace)?;
    let metadata = fs::metadata(&run.workspace).map_err(|_| ProviderRunError::InvalidRequest)?;
    if !metadata.is_dir() {
        return Err(ProviderRunError::InvalidRequest);
    }
    for (index, allowed) in run.allowed_paths.iter().enumerate() {
        let relative = Path::new(allowed);
        let normal: PathBuf = relative.components().collect();
        if allowed.is_empty()
            || !(allowed.ends_with(".md") || allowed.ends_with(".markdown"))
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            || normal.as_os_str() != relative.as_os_str()
            || run.allowed_paths[..index].contains(allowed)
        {
            return Err(ProviderRunError::InvalidRequest);
        }
        let target = run.workspace.join(relative);
        validate_workspace_target(&target)?;
        let metadata = fs::metadata(target).map_err(|_| ProviderRunError::InvalidRequest)?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(ProviderRunError::InvalidRequest);
        }
    }
    Ok(MarkdownWorkspaceIdentity {
        canonical: fs::canonicalize(&run.workspace)
            .map_err(|_| ProviderRunError::InvalidRequest)?,
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}
