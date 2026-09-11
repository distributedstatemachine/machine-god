use super::{
    MAX_MANAGED_SKILL_ENTRIES, MAX_MANAGED_SKILL_ITEMS, MAX_MANAGED_SKILL_TOTAL_BYTES,
    NativeManagedSkills, NativeSkillDestinationRevision, NativeSkillInstallItem,
    NativeSkillInstallPlan, NativeSkillInstallSource, NativeSkillManagedError,
    NativeSkillManagedErrorKind, NativeSkillSourceKind,
    filesystem::{self as fs, Budget, Tree},
    git, source,
};
use crate::skills_metadata::parse_skill_metadata;
use machine_god_core::CancellationToken;
use rustix::fs::Mode;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

#[path = "source_snapshot.rs"]
mod source_snapshot;

pub(super) struct SourceSnapshot {
    inventory: [u8; 32],
    selected: Vec<(String, [u8; 32])>,
}

struct Candidate {
    path: String,
    name: String,
    destination: String,
    directory: fs::Identity,
    directory_mode: Mode,
    file: fs::Identity,
    file_mode: Mode,
    digest: [u8; 32],
}
impl Candidate {
    fn matches(&self, tree: &Tree) -> bool {
        tree.root == self.directory
            && tree.root_mode == self.directory_mode
            && tree.entries.iter().any(|entry| {
                entry.path == "SKILL.md"
                    && entry.identity == self.file
                    && entry.mode == self.file_mode
                    && entry
                        .bytes
                        .as_ref()
                        .is_some_and(|bytes| <[u8; 32]>::from(Sha256::digest(bytes)) == self.digest)
            })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Operation {
    Install,
    Create,
    Remove,
}
pub(super) struct PlannedItem {
    pub name: String,
    pub destination: String,
    pub tree: Tree,
    pub expected: Option<NativeSkillDestinationRevision>,
    pub operation: Operation,
}
impl PlannedItem {
    pub fn summary(&self) -> NativeSkillInstallItem {
        NativeSkillInstallItem {
            name: self.name.clone(),
            destination: self.destination.clone(),
            replaces: self.expected.is_some(),
            entries: self.tree.entries.len(),
            bytes: self.tree.bytes(),
        }
    }
}
type Kind = NativeSkillManagedErrorKind;
type Error = NativeSkillManagedError;

pub(super) fn prepare_install(
    owner: &NativeManagedSkills,
    source: &NativeSkillInstallSource,
    cwd: &Path,
    cancellation: &CancellationToken,
) -> Result<NativeSkillInstallPlan, Error> {
    let mut budget = Budget::new(cancellation);
    budget.charge()?;
    let root_name = source_root_name(source, cwd)?;
    let (directory, lease) = match source.kind {
        NativeSkillSourceKind::Git => {
            let (directory, lease) = git::clone_source(owner, &source.source, cancellation)?;
            (directory, Some(lease))
        }
        NativeSkillSourceKind::Local => {
            let path = selected_local_path(Path::new(&source.source), cwd)?;
            let directory = fs::open_absolute_directory(&path)?;
            (Arc::new(directory), None)
        }
    };
    let result = prepare_from_directory(owner, source, &directory, &root_name, &mut budget);
    let source_retained = if lease.is_none() {
        Some(directory.try_clone().map_err(|_| Kind::Unavailable)?)
    } else {
        None
    };
    if let Some(lease) = lease {
        lease.cleanup()?;
    }
    let (items, original) = result?;
    Ok(NativeSkillInstallPlan {
        authority: Arc::clone(&owner.root),
        namespace: fs::named_identity(&owner.root, "skills")?,
        items,
        source: source_retained.map(|directory| (directory, original)),
    })
}

pub(super) fn source_root_name(
    source: &NativeSkillInstallSource,
    cwd: &Path,
) -> Result<String, Kind> {
    if source.kind == NativeSkillSourceKind::Git {
        let path = if source.source.contains("://") {
            url::Url::parse(&source.source)
                .map_err(|_| Kind::InvalidSource)?
                .path()
                .to_owned()
        } else {
            source
                .source
                .rsplit_once(':')
                .map_or(source.source.as_str(), |(_, path)| path)
                .to_owned()
        };
        let name = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("skill");
        let name = name.strip_suffix(".git").unwrap_or(name);
        source::validate_destination(name)?;
        return Ok(name.to_owned());
    }
    let selected = selected_local_path(Path::new(&source.source), cwd)?;
    let mut normalized = PathBuf::new();
    for component in selected.components() {
        if component == std::path::Component::ParentDir {
            normalized.pop();
        } else if component != std::path::Component::CurDir {
            normalized.push(component);
        }
    }
    let name = normalized
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Kind::InvalidName)?;
    source::validate_destination(name)?;
    Ok(name.to_owned())
}

fn selected_local_path<'a>(supplied: &'a Path, cwd: &Path) -> Result<Cow<'a, Path>, Kind> {
    let supplied_len = supplied.as_os_str().len();
    if supplied_len > 4096 {
        return Err(Kind::InvalidSource);
    }
    if supplied.is_absolute() {
        return Ok(Cow::Borrowed(supplied));
    }
    let cwd_bytes = cwd.as_os_str().as_encoded_bytes();
    let separator = usize::from(!cwd_bytes.is_empty() && !cwd_bytes.ends_with(b"/"));
    if cwd_bytes.len() > 4096
        || cwd_bytes
            .len()
            .checked_add(separator)
            .and_then(|length| length.checked_add(supplied_len))
            .is_none_or(|length| length > 4096)
    {
        return Err(Kind::InvalidSource);
    }
    // Bound the original spelling before joining or normalizing dot/parent components.
    // Only a relative local source uses the explicitly supplied working directory.
    Ok(Cow::Owned(cwd.join(supplied)))
}

fn prepare_from_directory(
    owner: &NativeManagedSkills,
    source: &NativeSkillInstallSource,
    directory: &File,
    root_name: &str,
    budget: &mut Budget<'_>,
) -> Result<(Vec<PlannedItem>, SourceSnapshot), Kind> {
    let mut selections = Vec::new();
    let inventory = fs::read_inventory(directory, budget, |observed| {
        let (parent, _) = observed
            .path
            .rsplit_once('/')
            .unwrap_or(("", observed.path));
        std::str::from_utf8(observed.bytes).map_err(|_| Kind::InvalidMetadata)?;
        let destination = if parent.is_empty() {
            root_name
        } else {
            parent.rsplit('/').next().ok_or(Kind::InvalidName)?
        };
        let metadata =
            parse_skill_metadata(observed.bytes, destination).map_err(|_| Kind::InvalidMetadata)?;
        if source
            .filter
            .as_deref()
            .is_some_and(|filter| filter != destination && filter != metadata.name)
        {
            return Ok(());
        }
        source::validate_destination(destination)?;
        if selections.len() >= MAX_MANAGED_SKILL_ITEMS {
            return Err(Kind::ResourceLimit);
        }
        selections.push(Candidate {
            path: parent.to_owned(),
            name: metadata.name,
            destination: destination.to_owned(),
            directory: observed.directory,
            directory_mode: observed.directory_mode,
            file: observed.file,
            file_mode: observed.file_mode,
            digest: Sha256::digest(observed.bytes).into(),
        });
        Ok(())
    })?;
    if selections.is_empty() {
        return Err(Kind::NoMatches);
    }
    let mut collisions = BTreeSet::new();
    for candidate in &selections {
        // Conservative Unicode lowercase plus the special long-s fold covers
        // ordinary native aliases without selecting arbitrary walk order.
        let folded = candidate
            .destination
            .chars()
            .flat_map(char::to_lowercase)
            .map(|c| if c == '\u{017f}' { 's' } else { c })
            .collect::<String>();
        if !collisions.insert(folded) {
            return Err(Kind::Collision);
        }
    }
    // Parent paths precede their descendants, allowing overlapping captures to share bytes.
    selections.sort_by(|a, b| a.path.cmp(&b.path));
    // Inventory and selected-resource reads are separate bounded phases. Candidate
    // bodies have already been discarded; their read budget does not reduce copy capacity.
    let captured = source_snapshot::capture(
        directory,
        selections.iter().map(|candidate| candidate.path.as_str()),
        &mut Budget::new(budget.cancellation),
    )?;
    let original = SourceSnapshot {
        inventory,
        selected: captured
            .iter()
            .map(|capture| (capture.path.clone(), capture.tree.fingerprint()))
            .collect(),
    };
    let mut destination_budget = Budget::new(budget.cancellation);
    let mut items = Vec::new();
    for (candidate, capture) in selections.into_iter().zip(captured) {
        if !candidate.matches(&capture.tree) {
            return Err(Kind::Changed);
        }
        let expected = destination_tree(owner, &candidate.destination, &mut destination_budget)?
            .map(|tree| revision(&candidate.destination, &tree));
        items.push(PlannedItem {
            name: candidate.name,
            destination: candidate.destination,
            tree: capture.tree,
            expected,
            operation: Operation::Install,
        });
    }
    items.sort_by(|a, b| a.destination.cmp(&b.destination));
    Ok((items, original))
}

pub(super) fn destination_tree(
    owner: &NativeManagedSkills,
    name: &str,
    budget: &mut Budget<'_>,
) -> Result<Option<Tree>, Kind> {
    budget.charge()?;
    if fs::named_identity(&owner.root, "skills")?.is_none() {
        return Ok(None);
    }
    let parent = fs::open_directory(&owner.root, "skills")?;
    if fs::named_identity(&parent, name)?.is_none() {
        return Ok(None);
    }
    let directory = fs::open_directory(&parent, name)?;
    let tree = fs::read_tree(&directory, false, budget)?;
    fs::verify_named(&parent, name, &directory)?;
    fs::verify_named(&owner.root, "skills", &parent)?;
    Ok(Some(tree))
}
pub(super) fn revision(name: &str, tree: &Tree) -> NativeSkillDestinationRevision {
    NativeSkillDestinationRevision {
        destination: name.to_owned(),
        fingerprint: tree.fingerprint(),
    }
}

pub(super) fn prepare_create(
    owner: &NativeManagedSkills,
    name: &str,
    cancellation: &CancellationToken,
) -> Result<NativeSkillInstallPlan, Kind> {
    source::validate_destination(name)?;
    let previous = destination_tree(owner, name, &mut Budget::new(cancellation))?;
    let expected = previous.as_ref().map(|tree| revision(name, tree));
    let mut tree = previous.unwrap_or_else(|| Tree::empty(fs::Identity(0, 0)));
    // This is the pinned bounded frontmatter grammar, not generic YAML: the
    // parser strips the matching outer quotes without interpreting escapes.
    let quoted = format!("'{name}'");
    let content = format!(
        "---\nname: {quoted}\ndescription: Describe when this skill should activate\n---\n\n# {name}\n\nInstructions for this skill...\n"
    );
    let parsed =
        parse_skill_metadata(content.as_bytes(), name).map_err(|_| Kind::InvalidMetadata)?;
    if parsed.name != name {
        return Err(Kind::InvalidMetadata);
    }
    tree.entries.retain(|entry| entry.path != "SKILL.md");
    if tree
        .entries
        .iter()
        .any(|entry| entry.path.starts_with("SKILL.md/"))
    {
        return Err(Kind::InvalidEntry);
    }
    tree.entries.push(fs::Entry {
        path: "SKILL.md".to_owned(),
        identity: fs::Identity(0, 0),
        mode: rustix::fs::Mode::from_raw_mode(0o600),
        bytes: Some(content.into_bytes().into()),
    });
    tree.entries.sort_by(|a, b| a.path.cmp(&b.path));
    if tree.bytes() > MAX_MANAGED_SKILL_TOTAL_BYTES
        || tree.entries.len() > MAX_MANAGED_SKILL_ENTRIES
    {
        return Err(Kind::ResourceLimit);
    }
    Ok(NativeSkillInstallPlan {
        authority: Arc::clone(&owner.root),
        namespace: fs::named_identity(&owner.root, "skills")?,
        source: None,
        items: vec![PlannedItem {
            name: name.to_owned(),
            destination: name.to_owned(),
            tree,
            expected,
            operation: Operation::Create,
        }],
    })
}
pub(super) fn prepare_remove(
    owner: &NativeManagedSkills,
    name: &str,
    cancellation: &CancellationToken,
) -> Result<NativeSkillInstallPlan, Kind> {
    source::validate_destination(name)?;
    let tree =
        destination_tree(owner, name, &mut Budget::new(cancellation))?.ok_or(Kind::NoMatches)?;
    let expected = Some(revision(name, &tree));
    Ok(NativeSkillInstallPlan {
        authority: Arc::clone(&owner.root),
        namespace: fs::named_identity(&owner.root, "skills")?,
        source: None,
        items: vec![PlannedItem {
            name: name.to_owned(),
            destination: name.to_owned(),
            tree: Tree::empty(tree.root),
            expected,
            operation: Operation::Remove,
        }],
    })
}
