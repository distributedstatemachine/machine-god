use super::{Kind, SourceSnapshot};
use crate::skills_managed::{
    MAX_MANAGED_SKILL_ENTRIES, MAX_MANAGED_SKILL_TOTAL_BYTES,
    filesystem::{self as fs, Budget, Tree},
};
use machine_god_core::CancellationToken;
use std::{fs::File, path::Path};

pub(super) struct CapturedTree {
    pub path: String,
    pub tree: Tree,
}

pub(super) fn capture<'a>(
    directory: &File,
    paths: impl Iterator<Item = &'a str>,
    budget: &mut Budget<'_>,
) -> Result<Vec<CapturedTree>, Kind> {
    let mut captured = Vec::new();
    let mut total_entries = 0_usize;
    let mut total_bytes = 0_usize;
    for path in paths {
        budget.charge()?;
        let tree = if let Some((ancestor, relative)) = ancestor(&captured, path) {
            ancestor.subset(relative)?
        } else {
            let selected = fs::open_path(directory, Path::new(path))?;
            fs::read_tree(&selected, true, budget)?
        };
        total_entries += tree.entries.len();
        total_bytes = total_bytes
            .checked_add(tree.bytes())
            .ok_or(Kind::ResourceLimit)?;
        if total_entries > MAX_MANAGED_SKILL_ENTRIES || total_bytes > MAX_MANAGED_SKILL_TOTAL_BYTES
        {
            return Err(Kind::ResourceLimit);
        }
        captured.push(CapturedTree {
            path: path.to_owned(),
            tree,
        });
    }
    Ok(captured)
}

fn ancestor<'a, 'b>(captured: &'a [CapturedTree], path: &'b str) -> Option<(&'a Tree, &'b str)> {
    captured.iter().rev().find_map(|capture| {
        let relative = if capture.path.is_empty() {
            path
        } else {
            path.strip_prefix(&capture.path)?.strip_prefix('/')?
        };
        if relative
            .split('/')
            .any(|component| component.starts_with(".git"))
        {
            return None;
        }
        Some((&capture.tree, relative))
    })
}

impl SourceSnapshot {
    pub(in crate::skills_managed) fn validate(
        &self,
        directory: &File,
        cancellation: &CancellationToken,
    ) -> Result<(), Kind> {
        let inventory = fs::read_inventory(directory, &mut Budget::new(cancellation), |_| Ok(()))?;
        if inventory != self.inventory {
            return Err(Kind::Changed);
        }
        let captured = capture(
            directory,
            self.selected.iter().map(|(path, _)| path.as_str()),
            &mut Budget::new(cancellation),
        )?;
        if captured
            .iter()
            .zip(&self.selected)
            .any(|(observed, (_, expected))| observed.tree.fingerprint() != *expected)
        {
            return Err(Kind::Changed);
        }
        Ok(())
    }
}
