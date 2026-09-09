use super::{
    NativeWorkspaceAction as Action, NativeWorkspaceReceipt as Receipt,
    NativeWorkspaceReconciliation as Reconciliation, NativeWorkspaceService as Service,
    NativeWorkspaceServiceError as Error, contain,
};
use crate::user_config_store::WorkspaceDirectoryAlias;
use crate::{
    NativeSavedWorkspaceDirectory as Saved, NativeWorkspaceCommitDurability as Durability,
    NativeWorkspaceDirectoryMutation as Mutation, NativeWorkspaceEntrySpec as Spec,
    NativeWorkspaceScopeSnapshot as Snapshot, NativeWorkspaceSource as Source,
};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

pub(super) fn run(
    service: &Service,
    action: Action,
    #[cfg(test)] hook: Option<&super::Hook>,
) -> Result<Receipt, Error> {
    #[cfg(test)]
    if let Some(hook) = hook {
        contain(|| hook(super::Stage::BeforeLoad)).map_err(|()| Error::Unavailable)?;
    }
    let previous = service.authority.snapshot().map_err(Error::Authority)?;
    if action == Action::List {
        let prepared = contain(|| service.authority.refresh_blocking())
            .map_err(|()| Error::Unavailable)?
            .map_err(Error::Authority)?;
        let snapshot = service
            .authority
            .install(prepared)
            .map_err(Error::Authority)?;
        return Ok(receipt(
            action,
            &previous,
            snapshot,
            Some(false),
            Reconciliation::Refreshed,
        ));
    }
    let aliases =
        contain(|| observed_aliases(service, &previous)).map_err(|()| Error::Unavailable)??;
    let (mutation, launch, _staged) =
        contain(|| stage(service, &previous, &action)).map_err(|()| Error::Unavailable)??;
    let commit = contain(|| {
        #[cfg(test)]
        if let Some(hook) = hook {
            hook(super::Stage::BeforeCommit);
        }
        futures_executor::block_on(
            service.store.apply_workspace_directory_mutation_observed(
                previous.primary_identity().as_os_str().as_bytes(),
                &mutation,
                &launch
                    .iter()
                    .map(|entry| entry.source().identity().as_os_str().as_bytes().to_vec())
                    .collect::<Vec<_>>(),
                &aliases,
            ),
        )
    })
    .map_err(|()| Error::Ambiguous)?
    .map_err(Error::Config)?;
    Ok(reconcile(
        service,
        action,
        &previous,
        &launch,
        &commit,
        &aliases,
        #[cfg(test)]
        hook,
    ))
}

pub(super) fn reconcile(
    service: &Service,
    action: Action,
    previous: &Snapshot,
    launch: &[Spec],
    commit: &crate::NativeUserWorkspaceCommit,
    aliases: &[WorkspaceDirectoryAlias],
    #[cfg(test)] hook: Option<&super::Hook>,
) -> Receipt {
    let saved_changed = (commit.durability == Durability::Confirmed).then_some(commit.changed);
    let loaded = contain(|| {
        #[cfg(test)]
        if let Some(hook) = hook {
            hook(super::Stage::AfterCommit);
        }
        service.store.load()
    })
    .map_err(|()| Error::Unavailable)
    .and_then(|result| result.map_err(Error::Config));
    let result = loaded.and_then(|loaded| {
        let saved = loaded
            .loaded()
            .config()
            .saved_workspace_directories(previous.primary_identity().as_os_str().as_bytes())
            .map_err(|error| Error::Config(crate::NativeUserConfigError::InvalidConfig(error)))?;
        let (launch, reconciliation, changed) = if commit.durability == Durability::Confirmed {
            (launch, Reconciliation::Confirmed, saved_changed)
        } else if saved == commit.after {
            (
                launch,
                Reconciliation::AmbiguousIntended,
                Some(commit.changed),
            )
        } else if saved == commit.before {
            // An observed previous saved set restores the exact pre-operation
            // launch provenance, not the removal/clear's staged survivors.
            return Ok(receipt(
                action.clone(),
                previous,
                previous.clone(),
                Some(false),
                Reconciliation::AmbiguousBefore,
            ));
        } else {
            return Ok(receipt(
                action.clone(),
                previous,
                previous.clone(),
                None,
                Reconciliation::Indeterminate,
            ));
        };
        let specs = super::sources::merge_observed_sources_blocking(
            saved,
            &launch
                .iter()
                .map(|entry| entry.source().clone())
                .collect::<Vec<_>>(),
            aliases,
        )?;
        let prepared = contain(|| {
            service
                .authority
                .prepare_blocking(specs, previous.saved_suppressed())
        })
        .map_err(|()| Error::Unavailable)?
        .map_err(Error::Authority)?;
        if previous.generation().checked_add(1) != Some(prepared.snapshot().generation()) {
            return Err(Error::Authority(
                crate::NativeWorkspaceAuthorityError::StaleGeneration,
            ));
        }
        #[cfg(test)]
        if let Some(hook) = hook {
            contain(|| hook(super::Stage::BeforeInstall)).map_err(|()| Error::Unavailable)?;
        }
        let snapshot = service
            .authority
            .install(prepared)
            .map_err(Error::Authority)?;
        Ok(receipt(
            action.clone(),
            previous,
            snapshot,
            changed,
            reconciliation,
        ))
    });
    match result {
        Ok(receipt) => receipt,
        Err(error) => receipt(
            action,
            previous,
            previous.clone(),
            saved_changed,
            Reconciliation::ReloadFailed(error),
        ),
    }
}

fn observed_aliases(
    service: &Service,
    previous: &Snapshot,
) -> Result<Vec<WorkspaceDirectoryAlias>, Error> {
    let loaded = service.store.load().map_err(Error::Config)?;
    let saved = loaded
        .loaded()
        .config()
        .saved_workspace_directories(previous.primary_identity().as_os_str().as_bytes())
        .map_err(|error| Error::Config(crate::NativeUserConfigError::InvalidConfig(error)))?;
    saved
        .iter()
        .filter(|record| !record.identity_canonical())
        .filter_map(|record| {
            previous
                .entries()
                .iter()
                .find(|entry| {
                    entry.spec().saved_record() == Some(record)
                        && entry.source().identity_canonical()
                })
                .map(|entry| {
                    WorkspaceDirectoryAlias::new(
                        record.clone(),
                        entry.source().identity().as_os_str().as_bytes(),
                    )
                    .map_err(Error::Config)
                })
        })
        .collect()
}

fn stage(
    service: &Service,
    previous: &Snapshot,
    action: &Action,
) -> Result<(Mutation, Vec<Spec>, crate::NativeWorkspacePreparedInstall), Error> {
    let mut specs: Vec<_> = previous
        .entries()
        .iter()
        .map(|entry| entry.spec().clone())
        .collect();
    let mutation = match action {
        Action::Add(path) => stage_add(service, previous, path, &mut specs)?,
        Action::Remove(path) => stage_remove(service, previous, path, &mut specs)?,
        Action::Clear => {
            specs.clear();
            Mutation::Clear
        }
        Action::List => return Err(Error::Unavailable),
    };
    let launch = specs
        .iter()
        .filter(|entry| entry.launch())
        .cloned()
        .collect();
    let prepared = match service
        .authority
        .prepare_blocking(specs, previous.saved_suppressed())
    {
        Ok(prepared) => prepared,
        Err(crate::NativeWorkspaceAuthorityError::TooManyDirectories)
            if matches!(action, Action::Add(_)) =>
        {
            let Mutation::Add(saved) = &mutation else {
                return Err(Error::Unavailable);
            };
            // Pinned capacity fallback: independently validate the requested root,
            // then let latest-under-lock storage assess the actual final union.
            service
                .authority
                .prepare_blocking(
                    merge(std::slice::from_ref(saved), &[])?,
                    previous.saved_suppressed(),
                )
                .map_err(Error::Authority)?
        }
        Err(error) => return Err(Error::Authority(error)),
    };
    Ok((mutation, launch, prepared))
}

fn stage_add(
    service: &Service,
    previous: &Snapshot,
    path: &Path,
    specs: &mut Vec<Spec>,
) -> Result<Mutation, Error> {
    validate_operand(path)?;
    let identity = std::fs::canonicalize(absolute(previous.primary_identity(), path))
        .map_err(|_| Error::InvalidPath)?;
    if identity == previous.primary_identity()
        || !std::fs::metadata(&identity)
            .map_err(|_| Error::InvalidPath)?
            .is_dir()
    {
        return Err(Error::InvalidPath);
    }
    let mut saved = Saved::new(
        identity.as_os_str().as_bytes(),
        identity.as_os_str().as_bytes(),
        true,
    )
    .map_err(|error| Error::Config(crate::NativeUserConfigError::InvalidConfig(error)))?;
    if let Some(entry) = specs
        .iter_mut()
        .find(|entry| entry.source().identity() == identity)
    {
        if entry.saved()
            && let Some(retained) = saved_source(service, previous, entry.source().source())?
        {
            saved = retained;
        }
        entry.include_saved_source();
    } else {
        specs.push(
            Spec::new(
                Source::new(identity.clone(), identity, true).map_err(Error::Authority)?,
                true,
                false,
            )
            .map_err(Error::Authority)?,
        );
    }
    Ok(Mutation::Add(saved))
}

fn stage_remove(
    service: &Service,
    previous: &Snapshot,
    path: &Path,
    specs: &mut Vec<Spec>,
) -> Result<Mutation, Error> {
    validate_operand(path)?;
    let input = lexical_absolute(previous.primary_identity(), path);
    let identity = specs
        .iter()
        .find(|entry| {
            lexical_absolute(previous.primary_identity(), entry.source().source()) == input
                || entry.source().identity() == input
        })
        .map(|entry| entry.source().identity().to_path_buf())
        .or_else(|| {
            std::fs::canonicalize(&input).ok().filter(|identity| {
                specs
                    .iter()
                    .any(|entry| entry.source().identity() == identity)
            })
        })
        .ok_or(Error::UnknownDirectory)?;
    let retained = specs
        .iter()
        .find(|entry| entry.saved() && entry.source().identity() == identity)
        .map(|entry| saved_source(service, previous, entry.source().source()))
        .transpose()?
        .flatten();
    specs.retain(|entry| entry.source().identity() != identity);
    Ok(Mutation::Remove(retained.map_or_else(
        || identity.as_os_str().as_bytes().to_vec(),
        |saved| saved.identity_bytes().to_vec(),
    )))
}

fn saved_source(
    service: &Service,
    previous: &Snapshot,
    source: &Path,
) -> Result<Option<Saved>, Error> {
    let loaded = service.store.load().map_err(Error::Config)?;
    let saved = loaded
        .loaded()
        .config()
        .saved_workspace_directories(previous.primary_identity().as_os_str().as_bytes())
        .map_err(|error| Error::Config(crate::NativeUserConfigError::InvalidConfig(error)))?;
    Ok(saved
        .iter()
        .find(|saved| saved.source_bytes() == source.as_os_str().as_bytes())
        .cloned())
}

fn merge(saved: &[Saved], launch: &[Spec]) -> Result<Vec<Spec>, Error> {
    super::merge_workspace_sources_blocking(
        saved,
        &launch
            .iter()
            .map(|entry| entry.source().clone())
            .collect::<Vec<_>>(),
    )
}

fn receipt(
    action: Action,
    previous: &Snapshot,
    snapshot: Snapshot,
    saved_changed: Option<bool>,
    reconciliation: Reconciliation,
) -> Receipt {
    let runtime_changed = previous.saved_suppressed() != snapshot.saved_suppressed()
        || previous.entries().len() != snapshot.entries().len()
        || previous
            .entries()
            .iter()
            .zip(snapshot.entries())
            .any(|(left, right)| {
                left.spec() != right.spec()
                    || left.available() != right.available()
                    || left.active() != right.active()
            });
    let launch_flag_can_restore = previous
        .entries()
        .iter()
        .filter(|entry| entry.launch())
        .any(|entry| {
            !snapshot
                .entries()
                .iter()
                .any(|new| new.launch() && new.source().identity() == entry.source().identity())
        });
    Receipt {
        action,
        snapshot,
        saved_changed,
        runtime_changed: Some(runtime_changed),
        launch_flag_can_restore,
        reconciliation,
    }
}

fn validate_operand(path: &Path) -> Result<(), Error> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() || bytes.len() > 4096 || bytes.contains(&0) {
        return Err(Error::InvalidPath);
    }
    Ok(())
}
fn absolute(primary: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        primary.join(path)
    }
}
fn lexical_absolute(primary: &Path, path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in absolute(primary, path).components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            component => result.push(component.as_os_str()),
        }
    }
    result
}
