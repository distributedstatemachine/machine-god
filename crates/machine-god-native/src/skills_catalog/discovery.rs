use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use machine_god_core::CancellationToken;
use rustix::{
    fd::AsFd,
    fs::{AtFlags, FileType},
};
use sha2::{Digest, Sha256};

use super::{
    MAX_NATIVE_SKILL_CANDIDATES, MAX_NATIVE_SKILL_DIAGNOSTICS, MAX_NATIVE_SKILL_MATERIALIZED_BYTES,
    MAX_NATIVE_SKILL_PATH_BYTES, MAX_NATIVE_SKILL_SNAPSHOT_BYTES, NativeSkillCatalog,
    NativeSkillCatalogError as Error, NativeSkillDiagnostic, NativeSkillEntry,
    NativeSkillMaterialized, NativeSkillRoot, NativeSkillSelection, NativeSkillSnapshot, Result,
    check,
    io::{self, Budget},
};
use crate::skills_metadata::parse_skill_metadata;

struct Scan {
    snapshot: NativeSkillSnapshot,
    budget: Budget,
    candidates: usize,
    text_bytes: usize,
    identities: BTreeSet<[i128; 2]>,
}

pub(super) fn discover(
    catalog: &NativeSkillCatalog,
    cancellation: &CancellationToken,
) -> Result<NativeSkillSnapshot> {
    let mut scan = Scan {
        snapshot: NativeSkillSnapshot {
            entries: Vec::new(),
            diagnostics: Vec::new(),
            complete: true,
            generation: [0; 32],
        },
        budget: Budget::default(),
        candidates: 0,
        text_bytes: 0,
        identities: BTreeSet::new(),
    };
    for root in catalog.roots.iter() {
        check(cancellation)?;
        if let Err(error) = scan.root(root, cancellation) {
            if error == Error::Cancelled {
                return Err(error);
            }
            if error == Error::NotFound {
                continue;
            }
            scan.diagnostic(root, root.display_path.clone(), true, error);
            if error == Error::ResourceLimit && scan.exhausted() {
                break;
            }
        }
    }
    scan.snapshot.generation = generation(catalog, &scan.snapshot);
    check(cancellation)?;
    Ok(scan.snapshot)
}

impl Scan {
    fn exhausted(&self) -> bool {
        self.budget.exhausted()
            || self.candidates >= MAX_NATIVE_SKILL_CANDIDATES
            || self.text_bytes >= MAX_NATIVE_SKILL_SNAPSHOT_BYTES
    }
    fn root(
        &mut self,
        root: &Arc<NativeSkillRoot>,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        let directory = io::open_directory(root, &root.relative, &mut self.budget, cancellation)?;
        let names = io::read_names(directory.as_fd(), &mut self.budget, cancellation)?;
        if names.invalid {
            self.diagnostic(root, root.display_path.clone(), true, Error::PathRejected);
        }
        for name in names.names {
            check(cancellation)?;
            if root.source == super::NativeSkillSource::Managed
                && name.starts_with(".machine-god-skill-")
            {
                continue;
            }
            let observed = self.budget.call(cancellation, || {
                rustix::fs::statat(&directory, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            });
            let observed = match observed {
                Ok(observed) => observed,
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(error) => {
                    self.diagnostic(root, root.display_path.join(&name), false, error);
                    if error == Error::ResourceLimit && self.exhausted() {
                        return Err(error);
                    }
                    continue;
                }
            };
            let kind = FileType::from_raw_mode(observed.st_mode);
            if !kind.is_dir() && !kind.is_symlink() {
                continue;
            }
            if self.candidates == MAX_NATIVE_SKILL_CANDIDATES {
                return Err(Error::ResourceLimit);
            }
            self.candidates += 1;
            match self.candidate(root, &name, cancellation) {
                Ok(()) | Err(Error::NotFound) => {}
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(error) => {
                    self.diagnostic(root, root.display_path.join(&name), false, error);
                    if error == Error::ResourceLimit && self.exhausted() {
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }

    fn candidate(
        &mut self,
        root: &Arc<NativeSkillRoot>,
        name: &str,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        let relative = root.relative.join(name);
        let location = root.display_path.join(name);
        if relative.as_os_str().len() > MAX_NATIVE_SKILL_PATH_BYTES
            || location.as_os_str().len() > MAX_NATIVE_SKILL_PATH_BYTES
        {
            return Err(Error::ResourceLimit);
        }
        let directory = io::open_directory(root, &relative, &mut self.budget, cancellation)?;
        let directory_identity =
            io::identity(&io::stat(&directory, &mut self.budget, cancellation)?);
        if self.identities.contains(&directory_identity) {
            return Ok(());
        }
        let (file, before) = io::open_file(directory.as_fd(), &mut self.budget, cancellation)?;
        let prefix = io::read_prefix(&file, &mut self.budget, cancellation)?;
        let after = io::stat(&file, &mut self.budget, cancellation)?;
        if io::revision(&before) != io::revision(&after) {
            return Err(Error::StaleSelection);
        }
        let metadata = parse_skill_metadata(&prefix, name).map_err(Error::InvalidMetadata)?;
        let retained_bytes = metadata.name.len()
            + metadata.description.len()
            + location.as_os_str().len()
            + relative.as_os_str().len();
        if self.text_bytes.saturating_add(retained_bytes * 2) > MAX_NATIVE_SKILL_SNAPSHOT_BYTES {
            return Err(Error::ResourceLimit);
        }
        self.text_bytes += retained_bytes * 2;
        self.identities.insert(directory_identity);
        let selection = NativeSkillSelection {
            root: Arc::clone(root),
            relative,
            location,
            metadata: metadata.clone(),
            directory_identity,
            revision: io::revision(&after),
            prefix_digest: Sha256::digest(&prefix).into(),
            prefix_bytes: prefix.len(),
        };
        self.snapshot.entries.push(NativeSkillEntry {
            metadata,
            selection,
        });
        Ok(())
    }

    fn diagnostic(
        &mut self,
        root: &NativeSkillRoot,
        location: PathBuf,
        is_root: bool,
        cause: Error,
    ) {
        self.snapshot.complete = false;
        if self.snapshot.diagnostics.len() == MAX_NATIVE_SKILL_DIAGNOSTICS {
            return;
        }
        let bytes = location.as_os_str().len();
        if bytes > MAX_NATIVE_SKILL_PATH_BYTES
            || self.text_bytes.saturating_add(bytes) > MAX_NATIVE_SKILL_SNAPSHOT_BYTES
        {
            return;
        }
        self.text_bytes += bytes;
        self.snapshot.diagnostics.push(NativeSkillDiagnostic {
            location,
            source: root.source,
            root: is_root,
            cause,
        });
    }
}

pub(super) fn materialize(
    selection: &NativeSkillSelection,
    cancellation: &CancellationToken,
) -> Result<NativeSkillMaterialized> {
    let mut budget = Budget::default();
    let directory = io::open_directory(
        &selection.root,
        &selection.relative,
        &mut budget,
        cancellation,
    )
    .map_err(stale_path)?;
    if io::identity(&io::stat(&directory, &mut budget, cancellation)?)
        != selection.directory_identity
    {
        return Err(Error::StaleSelection);
    }
    let (file, before) =
        io::open_file(directory.as_fd(), &mut budget, cancellation).map_err(stale_path)?;
    if io::revision(&before) != selection.revision {
        return Err(Error::StaleSelection);
    }
    if usize::try_from(before.st_size)
        .map_or(true, |size| size > MAX_NATIVE_SKILL_MATERIALIZED_BYTES)
    {
        return Err(Error::ResourceLimit);
    }
    let bytes = io::read_bytes(
        &file,
        MAX_NATIVE_SKILL_MATERIALIZED_BYTES,
        false,
        &mut budget,
        cancellation,
    )?;
    let after = io::stat(&file, &mut budget, cancellation)?;
    if io::revision(&after) != selection.revision {
        return Err(Error::StaleSelection);
    }
    let prefix = bytes
        .get(..selection.prefix_bytes)
        .ok_or(Error::StaleSelection)?;
    let digest: [u8; 32] = Sha256::digest(prefix).into();
    if digest != selection.prefix_digest {
        return Err(Error::StaleSelection);
    }
    let name = selection
        .relative
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Error::StaleSelection)?;
    let metadata = parse_skill_metadata(&bytes, name).map_err(Error::InvalidMetadata)?;
    if metadata != selection.metadata {
        return Err(Error::StaleSelection);
    }
    let digest = Sha256::digest(&bytes).into();
    let text = String::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)?;
    check(cancellation)?;
    Ok(NativeSkillMaterialized {
        text,
        metadata,
        selection: selection.clone(),
        digest,
    })
}

fn stale_path(error: Error) -> Error {
    match error {
        Error::NotFound | Error::PathRejected => Error::StaleSelection,
        other => other,
    }
}

fn generation(catalog: &NativeSkillCatalog, snapshot: &NativeSkillSnapshot) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"machine-god-skill-catalog-v1");
    hash.update([u8::from(snapshot.complete)]);
    for root in catalog.roots.iter() {
        hash.update(root.display_path.as_os_str().as_encoded_bytes());
        hash.update([0]);
        hash.update(format!("{:?}", root.source).as_bytes());
        hash.update([0]);
    }
    for entry in &snapshot.entries {
        let selection = &entry.selection;
        hash.update(selection.location.as_os_str().as_encoded_bytes());
        hash.update([0]);
        for value in selection
            .directory_identity
            .iter()
            .chain(selection.revision.iter())
        {
            hash.update(value.to_le_bytes());
        }
        hash.update(selection.prefix_digest);
    }
    hash.finalize().into()
}
