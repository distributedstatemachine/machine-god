//! Streaming parsing of the singleton, interactive-only recording modifier.

use super::{Command, parse_arguments};
use std::ffi::OsString;

pub(super) fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<(Command, bool), ()> {
    let mut arguments = RecordingArguments {
        inner: arguments.into_iter(),
        requested: false,
        invalid: false,
    };
    let command = parse_arguments(&mut arguments)?;
    // Some command parsers return early. Never let an unconsumed tail hide a
    // duplicate modifier or accidentally turn a one-shot prompt into recording.
    if arguments.next().is_some()
        || arguments.invalid
        || (arguments.requested && !matches!(command, Command::Interactive { .. }))
    {
        return Err(());
    }
    Ok((command, arguments.requested))
}

struct RecordingArguments<I> {
    inner: I,
    requested: bool,
    invalid: bool,
}

impl<I: Iterator<Item = OsString>> Iterator for RecordingArguments<I> {
    type Item = OsString;

    fn next(&mut self) -> Option<Self::Item> {
        for argument in self.inner.by_ref() {
            if argument == "--record" {
                self.invalid |= self.requested;
                self.requested = true;
            } else {
                self.invalid |= self.requested;
                return Some(argument);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InteractiveSessionSelection;

    #[test]
    fn recording_selects_fresh_and_existing_interactive_resume_grammar() {
        for arguments in [
            vec!["--record"],
            vec!["-r", "--record"],
            vec!["--resume", "--record"],
            vec!["--continue", "--record"],
            vec!["resume", "--record"],
            vec!["resume", "saved", "--record"],
            vec!["resume", "--id", "saved", "--record"],
            vec!["session", "resume", "saved", "--record"],
        ] {
            let (command, requested) = parse(arguments.into_iter().map(OsString::from)).unwrap();
            assert!(requested);
            assert!(matches!(command, Command::Interactive { .. }));
        }
        assert_eq!(
            parse([OsString::from("--record")]).unwrap(),
            (
                Command::Interactive {
                    selection: InteractiveSessionSelection::Fresh
                },
                true
            )
        );
    }

    #[test]
    fn misplaced_duplicate_and_noninteractive_recording_is_rejected() {
        for arguments in [
            vec!["--record", "resume"],
            vec!["resume", "--record", "saved"],
            vec!["--record", "--record"],
            vec!["resume", "--record", "--record"],
            vec!["ask", "question", "--record"],
            vec!["ask", "--", "--record"],
            vec!["resume", "saved", "question", "--record"],
            vec!["models", "--record"],
            vec!["session", "saved", "--record"],
            vec!["doctor", "--record"],
            vec!["workspace", "--record"],
        ] {
            assert!(parse(arguments.into_iter().map(OsString::from)).is_err());
        }
    }

    #[test]
    fn absent_modifier_preserves_prompt_tokens_without_an_argv_copy() {
        let (command, requested) = parse(["ask", "describe --record"].map(OsString::from)).unwrap();
        assert!(!requested);
        assert_eq!(
            command,
            Command::Ask {
                prompt: "describe --record".into()
            }
        );
        assert!(!parse(std::iter::empty()).unwrap().1);
    }
}
