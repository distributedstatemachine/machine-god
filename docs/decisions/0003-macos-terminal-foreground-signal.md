# ADR 0003: macOS PTY foreground signal binding

## Decision and scope

Use one audited native OS binding to close the interactive-terminal signaling
gap on macOS. The exception is limited to
`machine-god-terminal-sys::signal_terminal_foreground`, one `libc::ioctl` call
with the fixed `TIOCSIG` request. This is an internal component of the complete
terminal feature, subject to its local, adversarial and exact remote gates.

All existing production crates retain workspace `unsafe_code = "forbid"`.
The small workspace binding crate instead denies unsafe code, with an allowance
only on that function and denied undocumented unsafe blocks. It exposes a safe
borrowed-descriptor API, not arbitrary requests, raw pointers or PID authority.
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

Primary implementation references:

- [Apple request definition](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/ttycom.h)
- [Apple ioctl scalar marshaling](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/sys_generic.c)
- [Apple PTY signal dispatch](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/tty_dev.c)

## Integration and evidence

Native retains the owned PTY master and validates session ownership and signal
permission before calling this binding. Foreground signaling and close must
invoke it before closing the master. `TIOCSIG` can flush tty input/output; the
runtime must not promise that queued output survives a signal.

Tests cover non-terminal descriptors without consuming ownership, actual
foreground jobs with job control, and graceful/forced close. Binding and ABI
changes require adversarial inspection of this exact boundary and native tests
on macOS. Linux retains its pidfd-based lifecycle; no binding is built there.

This binding does not signal other background process groups. Session cleanup
must separately account for those groups and cannot declare success merely
because the foreground job and original shell have exited. That obligation
remains part of the full feature's acceptance gate.
