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
use std::collections::BTreeSet;
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

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
            let path = Path::new(&source.source);
            let directory = if path.is_absolute() {
                fs::open_absolute_directory(path)?
            } else {
                // Parent-relative spelling is resolved once as explicitly selected source authority.
                fs::open_absolute_directory(&cwd.join(path))?
            };
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
    let supplied = Path::new(&source.source);
    let selected = if supplied.is_absolute() {
        supplied.to_owned()
    } else {
        cwd.join(supplied)
    };
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

fn prepare_from_directory(
    owner: &NativeManagedSkills,
    source: &NativeSkillInstallSource,
    directory: &File,
    root_name: &str,
    budget: &mut Budget<'_>,
) -> Result<(Vec<PlannedItem>, Tree), Kind> {
    let original = fs::read_tree(directory, true, budget)?;
    let mut selections = Vec::new();
    for entry in &original.entries {
        let Some(bytes) = &entry.bytes else {
            continue;
        };
        let (parent, basename) = entry.path.rsplit_once('/').unwrap_or(("", &entry.path));
        if basename != "SKILL.md" {
            continue;
        }
        std::str::from_utf8(bytes).map_err(|_| Kind::InvalidMetadata)?;
        let destination = if parent.is_empty() {
            root_name
        } else {
            parent.rsplit('/').next().ok_or(Kind::InvalidName)?
        };
        let metadata =
            parse_skill_metadata(bytes, destination).map_err(|_| Kind::InvalidMetadata)?;
        if source
            .filter
            .as_deref()
            .is_some_and(|filter| filter != destination && filter != metadata.name)
        {
            continue;
        }
        source::validate_destination(destination)?;
        if selections.len() >= MAX_MANAGED_SKILL_ITEMS {
            return Err(Kind::ResourceLimit);
        }
        selections.push((
            metadata.name,
            destination.to_owned(),
            original.subset(parent)?,
        ));
    }
    if selections.is_empty() {
        return Err(Kind::NoMatches);
    }
    let mut collisions = BTreeSet::new();
    let mut total_bytes = 0_usize;
    let mut total_entries = 0_usize;
    for (_, destination, tree) in &selections {
        // Conservative Unicode lowercase plus the special long-s fold covers
        // ordinary native aliases without selecting arbitrary walk order.
        let folded = destination
            .chars()
            .flat_map(char::to_lowercase)
            .map(|c| if c == '\u{017f}' { 's' } else { c })
            .collect::<String>();
        if !collisions.insert(folded) {
            return Err(Kind::Collision);
        }
        total_bytes = total_bytes
            .checked_add(tree.bytes())
            .ok_or(Kind::ResourceLimit)?;
        total_entries += tree.entries.len();
    }
    if total_bytes > MAX_MANAGED_SKILL_TOTAL_BYTES || total_entries > MAX_MANAGED_SKILL_ENTRIES {
        return Err(Kind::ResourceLimit);
    }
    let mut destination_budget = Budget::new(budget.cancellation);
    let mut items = Vec::new();
    for (name, destination, tree) in selections {
        let expected = destination_tree(owner, &destination, &mut destination_budget)?
            .map(|tree| revision(&destination, &tree));
        items.push(PlannedItem {
            name,
            destination,
            tree,
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
