//! Thin ACP command entry. Native owns the protocol and session state.

pub(crate) fn run(
    host: &dyn crate::ask::AskCommandHost,
    stdout: &mut impl std::io::Write,
    stderr: &mut impl std::io::Write,
) -> u8 {
    crate::ask::finish_prompt_execution(
        host.execute_acp(stdout),
        stderr,
        "machine-god acp: connection failed\n",
        "machine-god acp: failed to write output\n",
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        Command,
        ask::{AskCommandExecution, AskCommandHost, AskCommandOutcome, SessionSelection},
    };
    use std::{cell::Cell, ffi::OsString, io};

    struct Host(Cell<usize>);
    impl AskCommandHost for Host {
        fn execute_acp(&self, output: &mut dyn io::Write) -> AskCommandExecution {
            self.0.set(self.0.get() + 1);
            let outcome = if output.write_all(b"wire\n").is_ok() {
                AskCommandOutcome::Completed
            } else {
                AskCommandOutcome::OutputFailure
            };
            AskCommandExecution::without_finalizer(outcome)
        }
        fn execute(
            &self,
            _: SessionSelection,
            _: String,
            _: &mut dyn io::Write,
        ) -> AskCommandExecution {
            panic!("ordinary prompt is not ACP")
        }
    }
    #[test]
    fn acp_command_has_no_legacy_or_argv_configuration_grammar() {
        assert_eq!(
            crate::parse_arguments([OsString::from("acp")]),
            Ok(Command::Acp)
        );
        for tail in ["--json", "--legacy", "--sse", "workspace", "--record"] {
            assert!(crate::parse_arguments(["acp".into(), tail.into()]).is_err());
        }
    }
    #[test]
    fn acp_frontend_only_invokes_selected_host() {
        let host = Host(Cell::new(0));
        let (mut output, mut error) = (Vec::new(), Vec::new());
        assert_eq!(super::run(&host, &mut output, &mut error), 0);
        assert_eq!(host.0.get(), 1);
        assert_eq!(output, b"wire\n");
        assert!(error.is_empty());
    }
}
