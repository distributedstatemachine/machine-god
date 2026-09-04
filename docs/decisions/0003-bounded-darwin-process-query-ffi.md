# ADR 0003: bounded Darwin process-query FFI

Status: accepted

## Context

Terminal process-tree signaling must identify a macOS process incarnation and
its parent lineage without trusting a reusable numeric PID. The required XNU
`PROC_PIDUNIQIDENTIFIERINFO` record exposes `p_uniqueid` and `p_puniqueid`, but
Apple's SDK omits the flavor constant and available safe Rust wrappers expose
only weaker start timestamps. Their PID-list helpers also allocate from a
kernel-reported count before the product can apply its resource limit.

Using those wrappers would either leave a PID-reuse compatibility gap or make
the documented process-snapshot bound untrue. Invoking an external helper
would add packaging, executable-discovery, filesystem, and subprocess
authority to every signal operation.

## Decision

Add `machine-god-darwin-proc`, a private, non-publishable workspace crate with
one module-local unsafe exception. Its public API is safe and limited to:

- listing all PIDs or one parent's direct children into a caller-owned fixed
  `i32` buffer, with a conservative truncation failure when Darwin fills it;
- reading one positive PID's BSD metadata between two flavor-17 identity
  reads; and
- returning fixed error categories rather than raw records or pointers.

The FFI module alone declares the three libproc calls and the two fixed
`repr(C)` records. Every call checks integer conversions and exact return
lengths. PID-list calls pass only the caller's initialized writable slice.
Process-info calls use `MaybeUninit`, assume initialization only after an exact
record-size return, and reject disagreement between the two unique-identity
reads. ABI size, alignment, and consumed-field offsets are asserted by macOS
tests. No pointer, uninitialized storage, raw errno, or FFI record crosses the
safe API.

All other workspace crates continue to forbid unsafe Rust. The wrapper crate
denies unsafe code by default and allows it only on its private macOS FFI
module with a reason. It may be linked only as a macOS-target dependency of
`machine-god-native`; core, the CLI, and the testkit cannot call it directly.

## Alternatives

- BSD start time plus numeric PID was rejected because it is weaker than the
  kernel process-incarnation identity used by the pinned compatibility target.
- `libproc` and equivalent crates were rejected because their safe APIs omit
  flavor 17 and allocate PID vectors before caller bounds apply.
- A runtime C or Swift helper was rejected because it expands packaging and
  process authority and adds avoidable launch overhead.
- Process-group-only delivery was rejected because descendants can enter a new
  group or session and survive a falsely successful signal action.

## Consequences

The production binary contains a small audited unsafe boundary on macOS. The
boundary depends on XNU flavor 17's stable record layout even though the SDK
does not publish its constant; an exact-size mismatch fails closed. macOS
updates therefore require native ABI tests before delivery.

Signal traversal can use unique process and parent identities with a fixed
caller allocation. Individual delivery still performs the same unavoidable
Darwin identity-check-then-`kill` sequence as the pinned implementation because
Darwin provides no pidfd-equivalent signal handle.

## Verification

- Format, lint, unit, doc, and native target gates include the wrapper crate.
- macOS tests assert both ABI layouts and query the current process and one
  live direct child through the safe API.
- Non-macOS tests prove the wrapper is inert and returns `Unsupported`.
- Dependency policy must show the wrapper only on the native macOS edge and
  must not retain the superseded `libproc` or `errno` dependencies.
- Product review must inspect every unsafe block, its input invariant, exact
  return check, and initialized-memory argument.
