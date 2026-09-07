//! Explicit, independently killable macOS PID inventory helper capability.
#![cfg(target_os = "macos")]

use crate::terminal_helper::{
    TerminalHelperError, TerminalHelperErrorKind, check_deadline, decode_helper_deadline,
    encode_helper_deadline,
};
use machine_god_core::CancellationToken;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write;
use std::num::NonZeroU32;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

#[path = "process_inventory_client.rs"]
mod client;
pub(crate) use client::InventoryLease;
#[cfg(test)]
#[path = "process_inventory_service_tests.rs"]
mod service_tests;
#[cfg(test)]
pub(crate) use client::RetirementHold;

/// Exact private mode; it is not an ordinary CLI command.
#[doc(hidden)]
pub const PROCESS_INVENTORY_HELPER_ARGUMENT: &str = "--machine-god-process-inventory-helper";
const DEADLINE_ENV: &str = "MACHINE_GOD_PROCESS_INVENTORY_DEADLINE";

#[derive(Clone, Debug)]
pub(crate) struct ProcessInventoryHelper {
    program: PathBuf,
    arguments: Vec<OsString>,
    service: Option<Arc<client::ServiceRegistration>>,
}

impl PartialEq for ProcessInventoryHelper {
    fn eq(&self, other: &Self) -> bool {
        self.program == other.program
            && self.arguments == other.arguments
            && self.service.is_some() == other.service.is_some()
    }
}
impl Eq for ProcessInventoryHelper {}

#[derive(Clone, Debug)]
pub(crate) enum PreparedProcessInventory {
    OneShot(ProcessInventoryHelper),
    Service(InventoryLease),
}

impl ProcessInventoryHelper {
    #[cfg(test)]
    pub(crate) fn service_spawn_count_for_test(&self) -> usize {
        self.service
            .as_ref()
            .unwrap()
            .starts
            .load(std::sync::atomic::Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn hold_retirement_for_test(&self) -> RetirementHold {
        self.service.as_ref().unwrap().hold_retirement_for_test()
    }
    pub(crate) fn new(
        program: PathBuf,
        arguments: Vec<OsString>,
    ) -> Result<Self, TerminalHelperError> {
        if !program.is_absolute()
            || program.as_os_str().as_bytes().len() > 4096
            || program.as_os_str().as_bytes().contains(&0)
            || arguments.len() > 16
            || arguments.iter().any(|argument| {
                argument.as_os_str().as_bytes().len() > 4096
                    || argument.as_os_str().as_bytes().contains(&0)
            })
        {
            return Err(failure(TerminalHelperErrorKind::InvalidRequest));
        }
        Ok(Self {
            program,
            arguments,
            service: None,
        })
    }

    pub(crate) fn new_service(
        program: PathBuf,
        arguments: Vec<OsString>,
    ) -> Result<Self, TerminalHelperError> {
        let mut helper = Self::new(program, arguments)?;
        helper.service = Some(Arc::new(client::ServiceRegistration::default()));
        Ok(helper)
    }

    pub(crate) fn rebind(&self) -> Self {
        let mut helper = self.clone();
        if helper.service.is_some() {
            helper.service = Some(Arc::new(client::ServiceRegistration::default()));
        }
        helper
    }

    pub(crate) fn prepare(
        &self,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<PreparedProcessInventory, TerminalHelperError> {
        self.prepare_with_stop(deadline, cancellation, &[])
    }

    pub(crate) fn prepare_with_stop(
        &self,
        deadline: Instant,
        cancellation: &CancellationToken,
        stop: &[&CancellationToken],
    ) -> Result<PreparedProcessInventory, TerminalHelperError> {
        check_deadline(deadline, cancellation)?;
        if stop.iter().any(|token| token.is_cancelled()) {
            return Err(failure(TerminalHelperErrorKind::Cancelled));
        }
        match &self.service {
            Some(service) => service
                .prepare(self, deadline, cancellation, stop)
                .map(PreparedProcessInventory::Service),
            None => Ok(PreparedProcessInventory::OneShot(self.clone())),
        }
    }

    pub(crate) fn command(&self, deadline: Instant) -> Result<Command, TerminalHelperError> {
        let stamp =
            encode_helper_deadline(deadline, crate::background_process::GROUP_SNAPSHOT_TIMEOUT)?;
        let mut command = Command::new(&self.program);
        command
            .args(&self.arguments)
            .env_clear()
            .env(DEADLINE_ENV, stamp);
        Ok(command)
    }
}

const fn failure(kind: TerminalHelperErrorKind) -> TerminalHelperError {
    TerminalHelperError { kind }
}

/// Collects untrusted PID hints in an independently owned helper process.
/// Hosts dispatch only the exact private flag, before creating ordinary workers.
/// No PID returned by this protocol is identity or session authority.
///
/// # Errors
/// Returns a fixed failure for an absent/expired deadline, incomplete inventory,
/// malformed or excessive output, or an output failure. No partial inventory is
/// a successful response.
#[doc(hidden)]
pub fn run_process_inventory_helper() -> Result<(), TerminalHelperError> {
    run_with_output(&mut std::io::stdout().lock())
}

fn run_with_output(output: &mut impl Write) -> Result<(), TerminalHelperError> {
    let stamp = std::env::var(DEADLINE_ENV)
        .map_err(|_| failure(TerminalHelperErrorKind::InvalidRequest))?;
    let deadline =
        decode_helper_deadline(&stamp, crate::background_process::GROUP_SNAPSHOT_TIMEOUT)?;
    let cancellation = CancellationToken::new();
    check_deadline(deadline, &cancellation)?;
    // ADR 0004: the potentially blocking query stays in this killable helper,
    // never in the parent's inventory collector or identity-scan worker.
    let pids = machine_god_terminal_sys::process_ids()
        .map_err(|_| failure(TerminalHelperErrorKind::Process))?;
    check_deadline(deadline, &cancellation)?;
    let bytes = encode_inventory(&pids)?;
    check_deadline(deadline, &cancellation)?;
    output
        .write_all(&bytes)
        .and_then(|()| output.flush())
        .map_err(|_| failure(TerminalHelperErrorKind::Process))?;
    check_deadline(deadline, &cancellation)
}

pub(crate) fn encode_inventory(pids: &[NonZeroU32]) -> Result<Vec<u8>, TerminalHelperError> {
    if pids.is_empty() || pids.len() > crate::background_process::MAX_CAPTURED_GROUP_MEMBERS {
        return Err(failure(TerminalHelperErrorKind::Protocol));
    }
    let mut seen = HashSet::with_capacity(pids.len());
    let mut output = String::with_capacity(crate::background_process::MAX_GROUP_SNAPSHOT_BYTES);
    for pid in pids {
        if pid.get() > i32::MAX.cast_unsigned() || !seen.insert(*pid) {
            return Err(failure(TerminalHelperErrorKind::Protocol));
        }
        // An i32 PID and newline need at most eleven bytes. Validate the
        // complete encoding before any write to the inherited output pipe.
        let line_bytes = usize::try_from(pid.get().ilog10()).expect("PID decimal width fits") + 2;
        if output.len().saturating_add(line_bytes)
            > crate::background_process::MAX_GROUP_SNAPSHOT_BYTES
        {
            return Err(failure(TerminalHelperErrorKind::Protocol));
        }
        writeln!(&mut output, "{pid}").expect("String formatting is infallible");
    }
    Ok(output.into_bytes())
}

pub(crate) fn decode_inventory(
    bytes: &[u8],
) -> Result<Vec<rustix::process::Pid>, TerminalHelperError> {
    let malformed = || failure(TerminalHelperErrorKind::Protocol);
    if bytes.is_empty() || bytes.len() > crate::background_process::MAX_GROUP_SNAPSHOT_BYTES {
        return Err(malformed());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
    let text = text.strip_suffix('\n').ok_or_else(malformed)?;
    let mut pids = Vec::new();
    let mut seen = HashSet::new();
    for line in text.split('\n') {
        if line.is_empty()
            || line.starts_with('0')
            || !line.bytes().all(|byte| byte.is_ascii_digit())
            || pids.len() == crate::background_process::MAX_CAPTURED_GROUP_MEMBERS
        {
            return Err(malformed());
        }
        let raw = line.parse::<i32>().map_err(|_| malformed())?;
        let pid = rustix::process::Pid::from_raw(raw).ok_or_else(malformed)?;
        if !seen.insert(pid) {
            return Err(malformed());
        }
        pids.push(pid);
    }
    Ok(pids)
}

#[cfg(test)]
pub(crate) fn test_helper() -> ProcessInventoryHelper {
    if let Some(program) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
        return ProcessInventoryHelper::new(
            program.into(),
            vec![PROCESS_INVENTORY_HELPER_ARGUMENT.into()],
        )
        .unwrap();
    }
    // This factory explicitly registers the matching test entrypoint. The
    // test harness's stdout is discarded; only its inherited stderr carries
    // the helper protocol, and the shell exec preserves direct-child ownership.
    let program = std::env::current_exe().unwrap();
    let script = format!(
        "exec '{}' --exact process_inventory_helper::tests::helper_entry --nocapture 2>&1 1>/dev/null",
        program.to_str().unwrap().replace('\'', "'\\''")
    );
    ProcessInventoryHelper::new("/bin/sh".into(), vec!["-c".into(), script.into()]).unwrap()
}

#[cfg(test)]
pub(crate) fn test_service() -> ProcessInventoryHelper {
    if let Some(program) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
        return ProcessInventoryHelper::new_service(
            program.into(),
            vec![crate::PROCESS_INVENTORY_SERVICE_ARGUMENT.into()],
        )
        .unwrap();
    }
    let program = std::env::current_exe().unwrap();
    let script = format!(
        "exec '{}' --exact process_inventory_protocol::tests::service_entry --nocapture 2>&1 1>/dev/null",
        program.to_str().unwrap().replace('\'', "'\\''")
    );
    ProcessInventoryHelper::new_service("/bin/sh".into(), vec!["-c".into(), script.into()]).unwrap()
}

#[cfg(test)]
pub(crate) fn stalled_service_for_test() -> ProcessInventoryHelper {
    service_tests::controlled_helper("startup_stalled")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_entry() {
        if std::env::var_os(DEADLINE_ENV).is_none() {
            return;
        }
        let result = run_with_output(&mut std::io::stderr().lock());
        std::process::exit(if result.is_ok() { 0 } else { 125 });
    }

    #[test]
    fn inventory_encoding_is_complete_bounded_and_unambiguous() {
        let pid = |value| NonZeroU32::new(value).unwrap();
        assert_eq!(encode_inventory(&[pid(1), pid(42)]).unwrap(), b"1\n42\n");
        assert!(encode_inventory(&[]).is_err());
        assert!(encode_inventory(&[pid(1), pid(1)]).is_err());
        assert!(encode_inventory(&[pid(u32::MAX)]).is_err());
        assert!(encode_inventory(&(1..=12_774).map(pid).collect::<Vec<_>>()).is_err());
        assert_eq!(
            encode_inventory(&(1..=12_773).map(pid).collect::<Vec<_>>())
                .unwrap()
                .len(),
            65_532
        );
        assert_eq!(decode_inventory(b"1\n42\n").unwrap().len(), 2);
        for bytes in [
            b"".as_slice(),
            b"1",
            b"1\n2",
            b"1\n1\n",
            b"\n",
            b"0\n",
            b"01\n",
            b"-1\n",
            b" 1\n",
            b"1\n\n",
            b"2147483648\n",
            b"\xff\n",
        ] {
            assert!(decode_inventory(bytes).is_err(), "{bytes:?}");
        }
        assert!(decode_inventory(&vec![b'1'; 65_537]).is_err());
    }

    #[test]
    fn inventory_helper_spec_is_explicit_and_bounded() {
        for program in ["relative", "", "/bad\0path"] {
            assert!(ProcessInventoryHelper::new(program.into(), vec![]).is_err());
        }
        assert!(ProcessInventoryHelper::new("/bin/false".into(), vec!["x".into(); 17]).is_err());
        assert!(
            ProcessInventoryHelper::new("/bin/false".into(), vec!["x".repeat(4097).into()])
                .is_err()
        );
        assert!(ProcessInventoryHelper::new("/bin/false".into(), vec!["\0".into()]).is_err());
        let helper = ProcessInventoryHelper::new("/bin/false".into(), vec![]).unwrap();
        assert!(helper.command(Instant::now()).is_err());
        let terminal =
            crate::terminal_helper::TerminalPtyHelper::new("/bin/true".into(), vec![]).unwrap();
        assert!(terminal.inventory_helper().is_none());
        let terminal = terminal.with_inventory_helper(helper.clone());
        assert_eq!(terminal.clone().inventory_helper(), Some(&helper));
    }
}
