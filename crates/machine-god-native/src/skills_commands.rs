//! Effect-free routing for the interactive `/skills` payload.
//!
//! Parsing selects a native operation, not filesystem or process authority.
//! Domain adapters validate selectors and installation/creation arguments before
//! observation or mutation. In particular, pasted package-manager syntax is
//! data for the installer parser, never a command to execute.

use std::{fmt, str::FromStr};

/// Maximum UTF-8 payload bytes, checked before scanning or allocation.
pub const MAX_NATIVE_SKILLS_COMMAND_BYTES: usize = 8 * 1024;
/// Maximum bytes in a show/remove selector; the catalog applies its own bounds.
pub const MAX_NATIVE_SKILLS_SELECTOR_BYTES: usize = 4 * 1024;

/// An inert interactive operation parsed without the `/skills` prefix.
///
/// Verbs are exact lowercase words, separated by ASCII spaces or tabs. Selectors
/// retain their interior bytes, including spaces; this layer does not interpret
/// shell quoting, locations, flags, source URLs, or replacement consent.
#[derive(Clone, Eq, PartialEq)]
pub enum NativeSkillsCommand {
    /// Discover and present the admitted catalog.
    List,
    /// Read one exact catalog selection after native resolution.
    Show { selector: String },
    /// Present the configured native managed location without creating it.
    Path,
    /// Validate creation arguments and consent through the managed adapter.
    Create { arguments: String },
    /// Validate source syntax, selection and consent through the managed adapter.
    Install { arguments: String },
    /// Resolve one managed selection before preparing removal.
    Remove { selector: String },
}

impl fmt::Debug for NativeSkillsCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::List => "NativeSkillsCommand::List",
            Self::Show { .. } => "NativeSkillsCommand::Show(..)",
            Self::Path => "NativeSkillsCommand::Path",
            Self::Create { .. } => "NativeSkillsCommand::Create(..)",
            Self::Install { .. } => "NativeSkillsCommand::Install(..)",
            Self::Remove { .. } => "NativeSkillsCommand::Remove(..)",
        })
    }
}

/// Fixed, content-free rejection of an invalid or oversized routing payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSkillsCommandError;

impl fmt::Display for NativeSkillsCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid native skills command")
    }
}

impl std::error::Error for NativeSkillsCommandError {}

impl FromStr for NativeSkillsCommand {
    type Err = NativeSkillsCommandError;

    fn from_str(payload: &str) -> Result<Self, Self::Err> {
        if payload.len() > MAX_NATIVE_SKILLS_COMMAND_BYTES
            || payload
                .chars()
                .any(|character| character.is_control() && character != '\t')
        {
            return Err(NativeSkillsCommandError);
        }
        let payload = payload.trim_matches([' ', '\t']);
        if payload.is_empty() || payload == "list" {
            return Ok(Self::List);
        }
        if payload == "path" {
            return Ok(Self::Path);
        }
        let (verb, arguments) = payload
            .split_once([' ', '\t'])
            .ok_or(NativeSkillsCommandError)?;
        let arguments = arguments.trim_matches([' ', '\t']);
        if arguments.is_empty() {
            return Err(NativeSkillsCommandError);
        }
        if matches!(verb, "show" | "remove") && arguments.len() > MAX_NATIVE_SKILLS_SELECTOR_BYTES {
            return Err(NativeSkillsCommandError);
        }
        match verb {
            "show" => Ok(Self::Show {
                selector: arguments.to_owned(),
            }),
            "create" => Ok(Self::Create {
                arguments: arguments.to_owned(),
            }),
            "add" | "install" => Ok(Self::Install {
                arguments: arguments.to_owned(),
            }),
            "remove" => Ok(Self::Remove {
                selector: arguments.to_owned(),
            }),
            _ => Err(NativeSkillsCommandError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_all_forms_without_interpreting_domain_arguments() {
        for input in ["", " \t ", "list", "\tlist  "] {
            assert_eq!(input.parse(), Ok(NativeSkillsCommand::List));
        }
        assert_eq!(" path\t".parse(), Ok(NativeSkillsCommand::Path));
        assert_eq!(
            "show\t日本語 skill  ".parse(),
            Ok(NativeSkillsCommand::Show {
                selector: "日本語 skill".into(),
            })
        );
        assert_eq!(
            "remove exact location".parse(),
            Ok(NativeSkillsCommand::Remove {
                selector: "exact location".into(),
            })
        );
        assert_eq!(
            "create example --replace".parse(),
            Ok(NativeSkillsCommand::Create {
                arguments: "example --replace".into(),
            })
        );
        for verb in ["add", "install"] {
            let arguments = "npx skills add owner/repo --skill example --replace";
            assert_eq!(
                format!("{verb} {arguments}").parse(),
                Ok(NativeSkillsCommand::Install {
                    arguments: arguments.into(),
                })
            );
        }
    }

    #[test]
    fn rejects_unknown_verbs_missing_arguments_and_list_path_arguments() {
        for input in [
            "List",
            "help",
            "/skills",
            "show",
            "remove\t",
            "create",
            "add",
            "install",
            "list extra",
            "path extra",
            "delete example",
            "list\n",
        ] {
            assert_eq!(
                input.parse::<NativeSkillsCommand>(),
                Err(NativeSkillsCommandError)
            );
        }
    }

    #[test]
    fn rejects_controls_before_allocating_command_arguments() {
        for control in ['\0', '\n', '\r', '\u{1b}', '\u{7f}', '\u{85}'] {
            let input = format!("show name{control}suffix");
            assert_eq!(
                input.parse::<NativeSkillsCommand>(),
                Err(NativeSkillsCommandError)
            );
        }
    }

    #[test]
    fn inclusive_byte_limits_apply_to_utf8_and_complete_payload() {
        let selector = "é".repeat(MAX_NATIVE_SKILLS_SELECTOR_BYTES / 2);
        assert!(
            format!("show {selector}")
                .parse::<NativeSkillsCommand>()
                .is_ok()
        );
        assert!(
            format!("remove {selector}x")
                .parse::<NativeSkillsCommand>()
                .is_err()
        );
        let input = format!("add {}", "x".repeat(MAX_NATIVE_SKILLS_COMMAND_BYTES - 4));
        assert!(input.parse::<NativeSkillsCommand>().is_ok());
        assert!(format!("{input} ").parse::<NativeSkillsCommand>().is_err());
    }

    #[test]
    fn debug_and_errors_do_not_disclose_arguments_or_locations() {
        for verb in ["show", "create", "install", "remove"] {
            let command: NativeSkillsCommand = format!("{verb} private-value").parse().unwrap();
            assert!(!format!("{command:?}").contains("private-value"));
        }
        assert_eq!(
            NativeSkillsCommandError.to_string(),
            "invalid native skills command"
        );
    }
}
