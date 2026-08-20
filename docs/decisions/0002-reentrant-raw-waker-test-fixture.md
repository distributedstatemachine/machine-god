# ADR 0002: test-only reentrant RawWaker fixture

Status: accepted

## Context

`CancellationToken` stores caller-provided `Waker` values. Raw-waker clone,
drop, and wake callbacks are user code and may synchronously reenter the token.
The safe `Wake` conversion uses fixed `Arc` bookkeeping for clone and drop, so it
cannot reproduce hostile raw clone/drop callbacks. The repository otherwise
forbids unsafe Rust.

## Decision

Keep `unsafe_code = "forbid"` unchanged for the workspace and every production
target. Isolate the minimum `RawWaker` construction in the excluded,
non-publishable `test-support/reentrant-waker` helper crate, referenced only as a
`machine-god-core` dev-dependency. Its public API is safe and accepts a callback
used solely by bounded lock-reentrancy unit tests.

The fixture's pointer ownership follows the `RawWakerVTable` contract:

- construction creates one raw `Arc` reference;
- clone preserves the borrowed raw reference with `ManuallyDrop` and creates
  exactly one new raw `Arc` reference;
- consuming wake and drop each recover and consume exactly one reference; and
- wake-by-reference recovers the reference under `ManuallyDrop` without
  consuming it.

The helper denies unsafe operations outside explicit unsafe blocks and enables
strict Clippy lints independently because it is excluded from the workspace.

## Consequences

Production artifacts cannot link the fixture, and the workspace unsafe-code
prohibition is not weakened. Focused validation must format and lint the helper
through its own manifest in addition to normal workspace gates.
