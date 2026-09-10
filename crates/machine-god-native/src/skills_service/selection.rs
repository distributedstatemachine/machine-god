use super::{NativeSkillsCatalogView, NativeSkillsNotice};
use crate::skills_catalog::{
    NativeSkillCatalogError as Error, NativeSkillEntry, NativeSkillSelection, NativeSkillSnapshot,
    NativeSkillSource,
};
use std::path::Path;

pub(super) fn show(snapshot: NativeSkillSnapshot, selector: &str) -> NativeSkillsCatalogView {
    let exact = Path::new(selector).is_absolute();
    let matches = snapshot
        .entries()
        .iter()
        .filter(|entry| {
            if exact {
                entry.location() == Path::new(selector)
            } else {
                entry.metadata.name == selector
            }
        })
        .collect::<Vec<_>>();
    let (focus, notice) = match matches.as_slice() {
        [entry] if exact || snapshot.complete() => (Some(entry.selection()), None),
        [] if snapshot.complete() => (None, Some(NativeSkillsNotice::NotFound)),
        [_, _, ..] => (None, Some(NativeSkillsNotice::Ambiguous)),
        _ => (None, Some(NativeSkillsNotice::IncompleteDiscovery)),
    };
    let query = if exact {
        matches
            .first()
            .map_or_else(String::new, |entry| entry.metadata.name.clone())
    } else {
        selector.to_owned()
    };
    NativeSkillsCatalogView {
        snapshot,
        query,
        focus,
        notice,
    }
}

pub(super) fn managed(
    snapshot: &NativeSkillSnapshot,
    selector: &str,
    managed_path: &Path,
) -> Result<NativeSkillSelection, Error> {
    let exact = Path::new(selector).is_absolute();
    if !exact && !snapshot.complete() {
        return Err(Error::IncompleteDiscovery);
    }
    let mut matches = snapshot
        .entries()
        .iter()
        .filter(|entry| is_managed(entry, managed_path))
        .filter(|entry| {
            if exact {
                entry.location() == Path::new(selector)
            } else {
                entry.metadata.name == selector
                    || entry
                        .location()
                        .file_name()
                        .is_some_and(|name| name == selector)
            }
        });
    let selected = matches.next().ok_or(Error::NotFound)?;
    if matches.next().is_some() {
        return Err(Error::AmbiguousName);
    }
    Ok(selected.selection())
}
fn is_managed(entry: &NativeSkillEntry, managed_path: &Path) -> bool {
    entry.source() == NativeSkillSource::Managed && entry.location().parent() == Some(managed_path)
}

#[cfg(test)]
thread_local! { static REMOVE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
thread_local! { static PREPARE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) fn set_remove_hook(hook: impl FnOnce() + 'static) {
    REMOVE_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}
#[cfg(test)]
pub(super) fn before_remove_revalidation() {
    REMOVE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}
#[cfg(test)]
pub(super) fn set_prepare_hook(hook: impl FnOnce() + 'static) {
    PREPARE_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}
#[cfg(test)]
pub(super) fn before_remove_preparation() {
    PREPARE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}
