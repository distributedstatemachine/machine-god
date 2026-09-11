//! Bounded reporting only; native owns selection, publication and recovery.

use crate::bounded_output::BoundedOutput;
use machine_god_native::{
    MAX_MANAGED_SKILL_ITEMS, MAX_NATIVE_SKILL_PATH_BYTES, MAX_NATIVE_SKILL_QUERY_ROWS,
    NativeSkillEntry, NativeSkillsCatalogView, NativeSkillsNotice, NativeSkillsServiceError,
    NativeSkillsServiceResult,
};
use std::fmt::Write;

pub(super) fn render(
    id: u64,
    result: Result<&NativeSkillsServiceResult, &NativeSkillsServiceError>,
) -> Result<Vec<u8>, ()> {
    let mut text = super::bounded_output();
    writeln!(text, "\n[control {id}: skills]").map_err(|_| ())?;
    match result {
        Err(error) => {
            writeln!(text, "{error}; no automatic retry").map_err(|_| ())?;
            if let NativeSkillsServiceError::Managed(error) = error {
                writeln!(text, "stage: {:?}", error.kind).map_err(|_| ())?;
                recovery(&mut text, error.recovery_id.as_deref())?;
            }
        }
        Ok(NativeSkillsServiceResult::Path(path)) => {
            if path.as_os_str().len() > MAX_NATIVE_SKILL_PATH_BYTES {
                return Err(());
            }
            super::presentation::escaped(&mut text, &path.to_string_lossy())?;
            text.write_char('\n').map_err(|_| ())?;
        }
        Ok(NativeSkillsServiceResult::Catalog(view)) => catalog(&mut text, view)?,
        Ok(NativeSkillsServiceResult::Managed(receipt)) => {
            if receipt.items.len() > MAX_MANAGED_SKILL_ITEMS {
                return Err(());
            }
            text.write_str("Destination previews may be clipped; outcomes are per item.\n")
                .map_err(|_| ())?;
            for item in &receipt.items {
                preview(&mut text, &item.destination, 192)?;
                write!(text, ": {:?}", item.outcome).map_err(|_| ())?;
                if let Some(error) = item.error {
                    write!(text, " ({error:?})").map_err(|_| ())?;
                }
                text.write_char('\n').map_err(|_| ())?;
                recovery(&mut text, item.recovery_id.as_deref())?;
            }
            if receipt.items.is_empty() || result.is_ok_and(NativeSkillsServiceResult::failed) {
                text.write_str(
                    "Batch incomplete or uncertain; inspect receipts before retrying.\n",
                )
                .map_err(|_| ())?;
            }
        }
    }
    text.write_str("> ").map_err(|_| ())?;
    Ok(text.finish().into_bytes())
}

fn catalog(text: &mut BoundedOutput, view: &NativeSkillsCatalogView) -> Result<(), ()> {
    if !view.snapshot.complete() {
        writeln!(
            text,
            "Discovery incomplete ({} diagnostics); names may be ambiguous.",
            view.snapshot.diagnostics().len()
        )
        .map_err(|_| ())?;
    }
    if let Some(notice) = view.notice {
        text.write_str(match notice {
            NativeSkillsNotice::NotFound => "Skill not found.\n",
            NativeSkillsNotice::Ambiguous => {
                "Multiple locations match; select an exact location.\n"
            }
            NativeSkillsNotice::IncompleteDiscovery => "No unique name selection is established.\n",
        })
        .map_err(|_| ())?;
    }
    if let Some(focus) = &view.focus {
        let entry = view
            .snapshot
            .entries()
            .iter()
            .find(|entry| entry.selection_ref() == focus)
            .ok_or(())?;
        text.write_str("Exact observed selection (preview):\n")
            .map_err(|_| ())?;
        row(text, entry)?;
    } else if view.query.len() <= machine_god_native::MAX_NATIVE_SKILL_QUERY_BYTES {
        let entries = view
            .snapshot
            .query(&view.query, MAX_NATIVE_SKILL_QUERY_ROWS)
            .map_err(|_| ())?;
        writeln!(text, "{} preview rows; catalog has {} entries. Names, descriptions and paths may be clipped.", entries.len(), view.snapshot.entries().len()).map_err(|_| ())?;
        for entry in entries {
            row(text, entry)?;
        }
        if view.snapshot.entries().len() > MAX_NATIVE_SKILL_QUERY_ROWS {
            text.write_str("Preview is bounded to 128 rows; use a more specific show selector.\n")
                .map_err(|_| ())?;
        }
    } else {
        text.write_str("Selector exceeds the menu query limit.\n")
            .map_err(|_| ())?;
    }
    Ok(())
}

fn row(text: &mut BoundedOutput, entry: &NativeSkillEntry) -> Result<(), ()> {
    preview(text, &entry.metadata.name, 96)?;
    text.write_str(" — ").map_err(|_| ())?;
    preview(text, &entry.metadata.description, 96)?;
    text.write_str("\n  ").map_err(|_| ())?;
    preview(text, &entry.location().to_string_lossy(), 192)?;
    text.write_char('\n').map_err(|_| ())
}

fn preview(text: &mut BoundedOutput, value: &str, bytes: usize) -> Result<(), ()> {
    let projected = super::composer_view::label(value, 240, bytes);
    text.write_str(std::str::from_utf8(&projected).map_err(|_| ())?)
        .map_err(|_| ())
}

fn recovery(text: &mut BoundedOutput, id: Option<&str>) -> Result<(), ()> {
    if let Some(id) = id {
        if id.len() > 128 {
            return Err(());
        }
        text.write_str("Recovery artifact retained: ")
            .map_err(|_| ())?;
        super::presentation::escaped(text, id)?;
        text.write_str("; manual inspection required, no automatic cleanup/retry.\n")
            .map_err(|_| ())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_native::{
        NativeSkillBatchReceipt, NativeSkillItemOutcome, NativeSkillItemReceipt,
        NativeSkillManagedError, NativeSkillManagedErrorKind,
    };

    #[test]
    fn partial_receipts_retain_all_outcomes_and_recovery_without_ansi_injection() {
        let receipt = NativeSkillsServiceResult::Managed(NativeSkillBatchReceipt {
            items: [
                NativeSkillItemOutcome::Installed,
                NativeSkillItemOutcome::Replaced,
                NativeSkillItemOutcome::Removed,
                NativeSkillItemOutcome::Failed,
                NativeSkillItemOutcome::RolledBack,
                NativeSkillItemOutcome::Indeterminate,
                NativeSkillItemOutcome::NotAttempted,
            ]
            .into_iter()
            .map(|outcome| NativeSkillItemReceipt {
                destination: "\x1b[31mprivate\nname".into(),
                outcome,
                error: Some(NativeSkillManagedErrorKind::Changed),
                recovery_id: Some(".machine-god-skill-recovery-123".into()),
            })
            .collect(),
        });
        let bytes = render(7, Ok(&receipt)).unwrap();
        assert!(!bytes.contains(&27));
        let text = String::from_utf8(bytes).unwrap();
        for expected in [
            "Installed",
            "Replaced",
            "Removed",
            "Failed",
            "RolledBack",
            "Indeterminate",
            "NotAttempted",
            ".machine-god-skill-recovery-123",
            "Batch incomplete or uncertain",
        ] {
            assert!(text.contains(expected), "{expected}");
        }
    }

    #[test]
    fn preparation_errors_preserve_opaque_recovery_receipt() {
        let error = NativeSkillsServiceError::Managed(NativeSkillManagedError::with_recovery(
            NativeSkillManagedErrorKind::Indeterminate,
            ".machine-god-skill-backup-123".into(),
        ));
        let text = String::from_utf8(render(9, Err(&error)).unwrap()).unwrap();
        assert!(text.contains("Indeterminate"));
        assert!(text.contains(".machine-god-skill-backup-123"));
        assert!(text.contains("no automatic cleanup/retry"));
    }

    #[test]
    fn maximum_batch_and_hostile_previews_remain_bounded() {
        let item = NativeSkillItemReceipt {
            destination: "\x1b[31m界".repeat(1024),
            outcome: NativeSkillItemOutcome::Installed,
            error: None,
            recovery_id: None,
        };
        let receipt = NativeSkillsServiceResult::Managed(NativeSkillBatchReceipt {
            items: vec![item.clone(); MAX_MANAGED_SKILL_ITEMS],
        });
        let bytes = render(1, Ok(&receipt)).unwrap();
        assert!(bytes.len() <= super::super::MAX_PRESENTATION_OUTPUT_BYTES);
        assert!(!bytes.contains(&27));
        let oversized = NativeSkillsServiceResult::Managed(NativeSkillBatchReceipt {
            items: vec![item; MAX_MANAGED_SKILL_ITEMS + 1],
        });
        assert!(render(1, Ok(&oversized)).is_err());
    }
}
