# ADR 0003: macOS identity-bound terminal control bindings

## Decision and scope

Use an isolated OS-binding crate to close interactive-terminal control gaps on
macOS. The exception is limited to four fixed calls:

- `signal_terminal_foreground`: one `libc::ioctl` with `TIOCSIG`;
- `ProcessIdentity::capture`: one fixed-size `proc_pidinfo` identity query;
- `ProcessIdentity::signal_token`: one fixed audit-token signaling operation
  through the declared `__proc_info` syscall wrapper;
- `uptime_raw`: one read-only `clock_gettime(CLOCK_UPTIME_RAW)` query for
  transferring the original Rust `Instant` deadline to a private helper.

These are internal components of the complete terminal feature, subject to
its local, adversarial and exact remote gates.

All existing production crates retain workspace `unsafe_code = "forbid"`.
The small workspace binding crate instead denies unsafe code, with an allowance
only on these calls and the necessary FFI declaration, and denied undocumented
unsafe blocks. It exposes safe borrowed-descriptor and opaque incarnation APIs,
not arbitrary requests, raw pointers or numeric-PID signal authority.
The clock accessor accepts no clock selector and returns only a validated
nonnegative `Duration` or an OS error.
This exception does not authorize unsafe process setup, `pre_exec`, session
enumeration, other ioctls, or other platform bindings.

## Rationale and ABI

An interactive shell places foreground jobs in separate process groups.
Signaling only its original group misses those jobs. Looking up a foreground
group number and subsequently calling `killpg` introduces a group-reuse race.
macOS `TIOCSIG` selects and signals the referenced foreground group under the
kernel tty lock. The pinned rustix version has no safe wrapper for this ioctl.

The request is `IOC_VOID`, despite taking a signal. Its argument is the signal
value, not a pointer to an integer. Pass a pointer-width scalar through the C
varargs ABI; XNU copies it into kernel-owned storage before interpreting the
signal. No Rust memory is dereferenced or retained. A borrowed descriptor
remains live for the synchronous call. The OS rejects unsuitable descriptors.
Read errno immediately on failure; do not retry an interrupted call, because
the foreground job may have changed. A successful call does not prove exit.

Background job-control groups need separate cleanup. The identity query uses
Apple's size-asserted 56-byte `proc_uniqidentifierinfo` ABI with initialized,
aligned storage. Its boot-local unique ID survives exec; its PID version
identifies the current executable incarnation. Audit-token signaling passes a
32-byte initialized token with PID/version selectors. The kernel references
and validates the exact target and separately checks the caller's credentials.
No token credential field grants privilege.

Native captures identity before and after checking membership in its retained,
unreaped shell's SID. A reused PID cannot become a cleanup target. Later exec
versions may be refreshed only while the unique ID matches. Signal delivery
validates that refreshed version in the kernel; bounded retries tolerate exec
races without falling back to `kill(PID)`. Identity queries alone never prove
ownership and persisted IDs never grant control.

Use the longstanding `__proc_info` wrapper with fixed operation `0x11` instead
of importing the newer `proc_signal_with_audittoken` symbol. Older systems
remain loadable and an unsupported operation returns an explicit OS error;
there is no unsafe numeric-PID fallback. The signal call reads the bounded
token synchronously and does not retain its pointer. Before preparing any
PTY, native probes this operation with PID zero (an invalid audit-token target,
not a process-group selector). Only the expected ESRCH response admits startup;
unsupported kernels fail before a shell can be committed.

Rust 1.94.1 uses `CLOCK_UPTIME_RAW` for macOS `Instant`. Encoding a transferable
deadline against `CLOCK_MONOTONIC` and decoding it against `Instant` mixes clock
domains; their offset is not a stable conversion. The fixed clock accessor uses
initialized `timespec` storage, checks the return value and timestamp range,
and retains no pointer. Native samples the same clock domain on both sides of
helper transfer and conservatively orders samples so conversion cannot extend
the original deadline. Linux continues to use its existing safe monotonic-clock
binding. This exception does not permit a general clock API or clock mutation.

Primary implementation references:

- [Apple request definition](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/ttycom.h)
- [Apple ioctl scalar marshaling](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/sys_generic.c)
- [Apple PTY signal dispatch](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/tty_dev.c)
- [Apple identity structure and operation definitions](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info_private.h)
- [Apple audit-token wrapper](https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/wrappers/libproc/libproc.c)
- [Apple identity validation and signal delivery](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/proc_info.c)

## Integration and evidence

Native retains the owned PTY master and validates session ownership and signal
permission before calling this binding. Foreground signaling and close must
invoke it before closing the master. `TIOCSIG` can flush tty input/output; the
runtime must not promise that queued output survives a signal.

Tests cover non-terminal/slave descriptors without consuming ownership, the
actual scalar PTY ABI, rejected stale PID versions/unique IDs, foreground jobs
with job control, and graceful/forced close of separate background/foreground
jobs ignoring HUP and TERM while an unrelated child remains untouched. Binding and ABI
changes require adversarial inspection of this exact boundary and native tests
on macOS. Linux retains its pidfd-based lifecycle; no binding is built there.

Native retains session-member identities across graceful termination and force
escalation, signals exact incarnations, and verifies quiescence before reaping
its owned shell. Resource, observation or delivery failures remain failures;
exiting the original shell alone is not evidence of complete session cleanup.
