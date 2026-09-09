//! Bounded presentation of native configured-rule observations and receipts.

use machine_god_native::{
    MAX_CONFIG_BYTES, NativeAllowlistReceipt, NativeAllowlistSources, NativeAllowlistView,
    NativeConfiguredPermissionMutation as Mutation,
    NativeConfiguredPermissionMutationOutcome as MutationOutcome,
    NativeConfiguredPermissionReset as Reset, NativeConfiguredPermissionRule,
    NativeConfiguredPermissionScope as Scope,
};
use std::{collections::HashMap, fmt, fmt::Write};

// Complete config input is capped at 64 KiB. Allow for terminal-safe Unicode
// expansion and headings without truncating a valid maximum-size rule list.
const MAX_OUTPUT_BYTES: usize = 8 * MAX_CONFIG_BYTES + 4096;

pub(super) fn render(id: u64, receipt: &NativeAllowlistReceipt) -> Result<Vec<u8>, ()> {
    let mut text = Output::default();
    writeln!(text, "\n[control {id}: allowlist]").map_err(|_| ())?;
    match receipt {
        NativeAllowlistReceipt::View {
            view,
            sources,
            reload,
        } => {
            render_view(&mut text, *view, sources)?;
            if reload.is_err() {
                text.write_str(
                    "runtime reload failed; displayed settings are not confirmed active\n",
                )
                .map_err(|_| ())?;
            }
        }
        NativeAllowlistReceipt::Mutation {
            scope,
            mutation,
            outcome,
            sources,
            reload,
        } => {
            render_mutation(&mut text, *scope, mutation, *outcome)?;
            if let Some(sources) = sources {
                if *scope == Scope::User
                    && !sources.user().rules().is_empty()
                    && sources.user_shadowed_by_local()
                {
                    text.write_str("; user rules shadowed by local workspace rules")
                        .map_err(|_| ())?;
                }
            } else if reload.is_some() {
                text.write_str("; effective source unknown")
                    .map_err(|_| ())?;
            }
            if reload.as_ref().is_some_and(Result::is_err) {
                text.write_str("; runtime reload failed").map_err(|_| ())?;
            }
            text.write_char('\n').map_err(|_| ())?;
        }
    }
    text.write_str("> ").map_err(|_| ())?;
    Ok(text.0.into_bytes())
}

fn render_view(
    text: &mut Output,
    view: NativeAllowlistView,
    sources: &NativeAllowlistSources,
) -> Result<(), ()> {
    let label = match view {
        NativeAllowlistView::Effective => "effective",
        NativeAllowlistView::Local => "local",
        NativeAllowlistView::User => "user",
    };
    let mut displayed = sources.display_rules(view).peekable();
    if displayed.peek().is_some() {
        writeln!(text, "{label} persistent allow rules:").map_err(|_| ())?;
        render_groups(text, displayed)?;
    } else {
        writeln!(text, "{label} persistent allow rules: (none)").map_err(|_| ())?;
    }
    if view != NativeAllowlistView::Local
        && !sources.user().rules().is_empty()
        && sources.user_shadowed_by_local()
    {
        text.write_str("user rules are shadowed by local workspace rules\n")
            .map_err(|_| ())?;
    }
    let rules = match view {
        NativeAllowlistView::Effective => Some(sources.effective()),
        NativeAllowlistView::Local => sources.local(),
        NativeAllowlistView::User => Some(sources.user()),
    };
    let warnings = rules.map_or(
        0,
        machine_god_native::NativeConfiguredPermissionRules::web_fetch_warning_count,
    );
    if warnings != 0 {
        writeln!(
            text,
            "ignored {warnings} malformed web_fetch rule{}; expected domain:<canonical-hostname>",
            if warnings == 1 { "" } else { "s" }
        )
        .map_err(|_| ())?;
    }
    Ok(())
}

struct Group<'a> {
    name: &'a str,
    patterns: Vec<&'a str>,
}

fn render_groups<'a>(
    text: &mut Output,
    rules: impl Iterator<Item = &'a NativeConfiguredPermissionRule>,
) -> Result<(), ()> {
    let mut groups: [Vec<Group<'a>>; 4] = std::array::from_fn(|_| Vec::new());
    let mut positions = HashMap::new();
    for rule in rules {
        let (section, name) = match rule.permission() {
            "bash" => (1, "command"),
            "url" => (2, "url"),
            "web_fetch" => (3, "web-fetch-domain"),
            name => (0, name),
        };
        let index = *positions.entry((section, name)).or_insert_with(|| {
            let index = groups[section].len();
            groups[section].push(Group {
                name,
                patterns: Vec::new(),
            });
            index
        });
        groups[section][index].patterns.push(rule.pattern());
    }
    for (section, groups) in groups.iter().enumerate() {
        if groups.is_empty() {
            continue;
        }
        writeln!(
            text,
            "  {}:",
            ["tools", "commands", "urls", "web-fetch domains"][section]
        )
        .map_err(|_| ())?;
        for group in groups {
            text.write_str("    ").map_err(|_| ())?;
            if section == 0 {
                escaped(text, group.name)?;
                text.write_str(": ").map_err(|_| ())?;
            }
            for (index, pattern) in group.patterns.iter().enumerate() {
                if index != 0 {
                    text.write_str(", ").map_err(|_| ())?;
                }
                if section == 0 && *pattern == "*" && workspace_tool(group.name) {
                    text.write_str("workspace").map_err(|_| ())?;
                } else {
                    escaped(text, pattern)?;
                }
            }
            text.write_char('\n').map_err(|_| ())?;
        }
    }
    Ok(())
}

fn workspace_tool(name: &str) -> bool {
    matches!(
        name,
        "edit"
            | "create_folder"
            | "open_file"
            | "rename_file"
            | "copy_file"
            | "read"
            | "list"
            | "glob"
            | "grep"
    )
}

fn render_mutation(
    text: &mut Output,
    scope: Scope,
    mutation: &Mutation,
    outcome: MutationOutcome,
) -> Result<(), ()> {
    let changed = matches!(outcome, MutationOutcome::Changed { .. });
    match mutation {
        Mutation::Add {
            permission,
            pattern,
        }
        | Mutation::Remove {
            permission,
            pattern,
        } => {
            let verb = match (mutation, changed) {
                (Mutation::Add { .. }, true) => "added",
                (Mutation::Add { .. }, false) => "already allowed",
                (_, true) => "removed",
                (_, false) => "no matching rule for",
            };
            write!(text, "{verb} ").map_err(|_| ())?;
            match permission.as_str() {
                "bash" => text.write_str("command"),
                "url" => text.write_str("url"),
                "web_fetch" => text.write_str("web-fetch-domain"),
                _ => {
                    text.write_str("tool ").map_err(|_| ())?;
                    escaped(text, permission)?;
                    Ok(())
                }
            }
            .map_err(|_| ())?;
            text.write_str(": \"").map_err(|_| ())?;
            escaped(text, pattern)?;
            text.write_char('"').map_err(|_| ())?;
        }
        Mutation::Reset(reset) => {
            let label = match reset {
                Reset::Commands => "commands",
                Reset::Tools => "tools",
                Reset::Urls => "urls",
                Reset::WebFetchDomains => "web-fetch-domains",
                Reset::All => "all",
            };
            let removed = match outcome {
                MutationOutcome::Unchanged => 0,
                MutationOutcome::Changed { removed_rules } => removed_rules,
            };
            write!(
                text,
                "reset {label}: removed {removed} rule{}",
                if removed == 1 { "" } else { "s" }
            )
            .map_err(|_| ())?;
        }
    }
    write!(
        text,
        " (scope={}); settings {}",
        match scope {
            Scope::User => "user",
            Scope::Local => "local",
        },
        if changed { "saved" } else { "unchanged" }
    )
    .map_err(|_| ())
}

fn escaped(text: &mut Output, value: &str) -> Result<(), ()> {
    crate::write_json_string_content(text, value).map_err(|_| ())
}

#[derive(Default)]
struct Output(String);
impl fmt::Write for Output {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self
            .0
            .len()
            .checked_add(value.len())
            .is_none_or(|len| len > MAX_OUTPUT_BYTES)
        {
            return Err(fmt::Error);
        }
        self.0.push_str(value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_native::{NativeConfiguredPermissionDecision, NativeConfiguredPermissionRules};

    fn rule(permission: &str, pattern: &str) -> NativeConfiguredPermissionRule {
        NativeConfiguredPermissionRule::new(
            permission,
            pattern,
            NativeConfiguredPermissionDecision::Allow,
        )
        .unwrap()
    }

    #[test]
    fn groups_preserve_first_appearance_and_pattern_order_with_safe_workspace_labels() {
        let rules = NativeConfiguredPermissionRules::new(vec![
            rule("url", "https://example.test/*"),
            rule("read", "*"),
            rule("skill", "*"),
            rule("read", "file\u{1b}[2J"),
            rule("bash", "git status"),
            rule("read", "*"),
            rule("web_fetch", "domain:example.test"),
            rule("*", "*"),
        ])
        .unwrap();
        let mut output = Output::default();
        render_groups(&mut output, rules.rules().iter()).unwrap();
        assert_eq!(
            output.0,
            "  tools:\n    read: workspace, file\\u001b[2J, workspace\n    skill: *\n    *: *\n  commands:\n    git status\n  urls:\n    https://example.test/*\n  web-fetch domains:\n    domain:example.test\n"
        );
    }

    #[test]
    fn escaped_rule_projection_can_exceed_model_output_cap_without_truncation() {
        let pattern = "\u{0085}".repeat(20_000);
        let rules = NativeConfiguredPermissionRules::new(vec![rule("skill", &pattern)]).unwrap();
        let mut output = Output::default();
        render_groups(&mut output, rules.rules().iter()).unwrap();
        assert!(
            output.0.len() > crate::ask::production::interactive::MAX_PRESENTATION_OUTPUT_BYTES
        );
        assert_eq!(
            output.0,
            format!("  tools:\n    skill: {}\n", "\\u0085".repeat(20_000))
        );
        assert!(output.0.len() < MAX_OUTPUT_BYTES);
    }

    #[test]
    fn output_cap_is_inclusive_and_rejection_does_not_publish_a_partial_append() {
        let mut output = Output::default();
        output.write_str(&"x".repeat(MAX_OUTPUT_BYTES)).unwrap();
        assert!(output.write_str("y").is_err());
        assert_eq!(output.0.len(), MAX_OUTPUT_BYTES);
        assert!(output.0.ends_with('x'));
    }

    #[test]
    fn mutation_receipts_distinguish_noops_saves_and_zero_removal_without_raw_controls() {
        let remove = Mutation::Remove {
            permission: "read".into(),
            pattern: "file\u{202e}".into(),
        };
        let mut output = Output::default();
        render_mutation(
            &mut output,
            Scope::Local,
            &remove,
            MutationOutcome::Unchanged,
        )
        .unwrap();
        assert_eq!(
            output.0,
            "no matching rule for tool read: \"file\\u202e\" (scope=local); settings unchanged"
        );
        let mut output = Output::default();
        render_mutation(
            &mut output,
            Scope::User,
            &Mutation::Reset(Reset::All),
            MutationOutcome::Changed { removed_rules: 2 },
        )
        .unwrap();
        assert_eq!(
            output.0,
            "reset all: removed 2 rules (scope=user); settings saved"
        );
        let mut output = Output::default();
        render_mutation(
            &mut output,
            Scope::Local,
            &Mutation::Reset(Reset::Tools),
            MutationOutcome::Unchanged,
        )
        .unwrap();
        assert_eq!(
            output.0,
            "reset tools: removed 0 rules (scope=local); settings unchanged"
        );
    }

    #[test]
    fn missing_remove_does_not_claim_reload_or_unknown_effective_source() {
        let receipt = NativeAllowlistReceipt::Mutation {
            scope: Scope::Local,
            mutation: Mutation::Remove {
                permission: "read".into(),
                pattern: "*".into(),
            },
            outcome: MutationOutcome::Unchanged,
            sources: None,
            reload: None,
        };
        let output = String::from_utf8(render(7, &receipt).unwrap()).unwrap();
        assert!(output.contains("no matching rule for tool read"));
        assert!(output.contains("settings unchanged"));
        assert!(!output.contains("unknown"));
        assert!(!output.contains("reload"));
        assert!(!output.contains("saved"));
    }
}
