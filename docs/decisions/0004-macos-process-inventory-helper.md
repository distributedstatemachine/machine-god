# ADR 0004: bounded macOS process inventory helper

## Decision

Permit one additional fixed read-only binding in `machine-god-terminal-sys`:
`process_ids() -> io::Result<Vec<NonZeroU32>>`, using only
`sysctl(CTL_KERN, KERN_PROC, KERN_PROC_ALL)`. It returns untrusted PID hints,
not process ownership, session membership or signal authority. The product
crates retain `unsafe_code = "forbid"`; the binding crate retains its denied
unsafe default and documented, call-local exception. ADR 0003's four existing
bindings remain unchanged. This decision does not authorize arbitrary sysctl
selectors, writes, process setup, pointer-bearing public APIs or numeric-PID
signal fallbacks.

Native invokes this operation in an explicitly configured private helper, not
in its collector thread. The reference host wires the executable capability
through PTY/captured-exec and tmux cleanup ownership. It never guesses that the
current executable implements the helper protocol. Legacy constructors without
that capability retain their existing `ps` behavior; group-only scans are also
unchanged. A selected helper's failure does not fall back to another inventory.

## Rationale and alternatives

The existing session-wide `/bin/ps -axo pid=` path performs Mach task/thread
inspection even though native needs only PID hints. Removing that extra work
addresses measured inventory overhead. It does not prove the exclusive cause
of every observed timeout or establish a product performance threshold.

The fixed sysctl uses the same kernel process-list family as `ps`, including
live and zombie processes. Its iterator sizes its internal PID storage while
holding the process-list lock, and the handler reports `ENOMEM` when caller
storage cannot hold the result. A size query is only a hint; neither a partial
result nor a failed query is a successful inventory. This is a changing process
view, not an atomic lifetime/ownership proof. Existing incarnation and session
checks, retained cleanup prefixes and final quiescence checks remain required.

`proc_listpids` is not substituted: its internal capacity can lag process-list
growth without exposing complete-inventory proof through the caller's buffer.
An in-process sysctl would remove launch overhead but could wait in the kernel
beyond the collector's cancellation deadline. A separately owned helper keeps
that wait outside the caller and preserves bounded kill/reap or quarantine.

## ABI and resource boundary

The supported macOS ARM64 and x86_64 ABI uses 648-byte `kinfo_proc` records with
a native-endian signed 32-bit PID at byte offset 40. The wrapper uses initialized
byte storage, not a Rust reference or cast to an invented C structure. Only
the fixed synchronous call receives buffer and length pointers; no pointer is
retained or exposed. SDK size/offset assertions verify both target layouts.
Unsupported layouts must not be added by silently reusing these constants.

Raw scratch is capped at 8 MiB. Size queries and at most three data attempts
allow bounded growth races; failed/partial data is discarded. Successful data
must fit the supplied storage and consist of complete records. Negative or
duplicate PIDs, invalid lengths and overflow fail closed; only the single
kernel PID-zero row may be omitted. The wrapper returns positive PIDs only.

The cap does not shrink the existing 64 KiB decimal PID-text admission: the
shortest possible 12,773 unique positive PID lines occupy 65,532 bytes; one
more needs 65,538. Those records plus PID zero occupy 8,277,552 raw bytes,
below 8 MiB. The helper validates and bounds its complete encoded output
before writing. PID/output vectors are separately bounded by the raw-record
and 64 KiB limits. Scratch is helper-local; the existing process-wide maximum
of 64 reap authorities also bounds simultaneous or quarantined helpers.

The parent retains nonblocking pipe collection, the 64 KiB output limit and
the original 250 ms collection/identity-scan deadline. Acceptance requires
both EOF and successful reaping. For the selected helper, the parent fixes this
deadline before spawn and transfers it in the existing conservative uptime
clock domain; the helper does not start a fresh budget. Errors preserve owned
cleanup and cannot
turn a partial prefix into a complete inventory. Kernel data is only a source
of candidates for the unchanged identity/session sandwich.

## Verification

Test malformed, oversized and truncated results; duplicate, negative and zero
PIDs; discarded `ENOMEM` prefixes and bounded retry exhaustion; live inventory;
exact private CLI dispatch; helper failure, output bounds and cleanup ownership.
Run PTY, captured-exec and tmux lifecycle tests with explicit test and production
helpers, and the full feature's exact pinned local, independent review and
Linux/macOS remote gates. This ADR does not waive those gates.

Primary implementation references:

- [Apple process handler and truncation reporting](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_sysctl.c#L809-L945)
- [Apple process-list allocation and iteration](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_proc.c#L4024-L4148)
- [Apple ps task/thread inspection](https://github.com/apple-oss-distributions/adv_cmds/blob/adv_cmds-231/ps/tasks.c#L90-L234)
