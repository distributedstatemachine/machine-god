# Milestone 03 native session-listing review 01

Status: **PORTABLE FIX REVIEW GREEN — REPLACEMENT REMOTE GATES PENDING**

All three first-round formal tracks reported confirmed findings on exact
candidate `dec98e06e110be88317a2ba76c77e7ca872fed34`. Production and the
initial independent suite are composed; that first candidate was not green, and
the slice remains undelivered until its remote gates pass.
Replacement production work, test hardening, and corrected documentation are
composed in this exact replacement candidate, with 18 focused tests and the
full all-target/all-feature workspace suite green locally. All three replacement
review tracks are green on exact behavior candidate
`3fa54635dab00ebba78b233c69fd39e04e9be57e`. Remote exact-SHA evidence remains
pending.

Per the user's instruction, a later documentation-only seal will not require a
new adversarial review after one exact behavior candidate is green. That does
not waive any delivery gate; the required replacement rereview is now complete.

## Exact lineage

| Stage | Exact commit |
| --- | --- |
| Integrated fifteen-slice base | `9ada4b5429b89138cc18aec2b8e88e610b1df2fb` |
| Isolated production | `0accfbfeba7aa0e1fa9865a5256440028d14da2c` |
| Composed production | `1bffac971fbeb1ee8ba885166a7d456d72ab2cb6` |
| Isolated documentation | `63d589ce3fdfc90683fb980cd41f7398ccaead8b` |
| Composed documentation | `87d7de0402a6fcce79344f45aa7d71fd196bc7a6` |
| Isolated independent tests | `1b531297c12147028217b31bd0e3dee20f139607` |
| Composed independent tests | `4b4e468380265cc52a0888c7ab33ecc104eebc25` |
| Removed-root finding fix and first formal candidate | `dec98e06e110be88317a2ba76c77e7ca872fed34` |
| Isolated acquire-first replacement production fix | `4b8d8b0b38e081d1fd5ada3ba969f5b9af94eead` |
| Isolated finding-test hardening | `446b49558b989edb399a8dabb33efe89beb490da` |
| Composed replacement behavior candidate | `3fa54635dab00ebba78b233c69fd39e04e9be57e` |
| Documentation correction head | `6689ab91d0d096eda9cf0aeb9dfb382887ff01dc` |
| First remote feature head | `a70a864673e4df6c7555c06008d43ad802285437` |
| Portable Linux remote-finding behavior candidate | `17f1884c20e84574561eb3cedd96b9aee6d37284` |

The composed test commit contains 13 initial independently owned focused tests.
The isolated test-hardening commit raises that focused suite to 18 tests, and
those 18 focused tests are green. The source and test fixes are composed in this
replacement candidate and are green under all three replacement review tracks.
They are not yet remote evidence.

## Candidate boundary

The Linux/macOS library slice adds `NativeSessionLifecycle::list_sessions`,
returning an IDs-only `NativeSessionList` and `truncated`. Its normative
replacement contract is
[`native-session-listing.md`](../native-session-listing.md).

One call returns no more than 100 sorted unique IDs. It processes or selects at
most 1,024 non-dot directory entries and may fetch and name-inspect one
additional non-dot entry only as the overflow witness. It accepts and decodes at
most 64 MiB of aggregate canonical record bytes and may transiently transfer
one additional byte only to detect concurrent growth beyond the remaining
budget. Every non-dot entry within the scan consumes scan budget. Candidate
filenames are sorted before validation, and only a fired raw scan cap makes the
selected candidate set filesystem-iteration-dependent.

Exact canonical candidates use the current strict file-session schema, the
same no-follow regular-file validation and per-ID locks, and the decoded-ID/
digest check. A nonregular derived lock for a present canonical record is
`Corrupt`; ordinary lock I/O is `Unavailable`. Returned IDs and
`NativeSessionList`'s derived `Debug` deliberately expose session identities.
Only lifecycle error `Display` and `Debug` are ID/path/content-redacted.

The replacement macOS liveness order is normative: first acquire a fresh `.`
descriptor, then validate the linked identity of that exact acquired
descriptor. A stable completed rename retains identity. Removal before
acquisition or identity validation is `Unavailable`. A concurrent rename or
removal may conservatively yield `Unavailable` or an observation of the exact
acquired identity; neither outcome is a global snapshot or permission to follow
a replacement.

The slice adds no live-registry/source/provider/tool or network authority, rich
summaries, workspace/latest/cursor semantics, CLI surface, compatibility claim,
benchmark implementation, or product-performance claim. Its future is inert
before polling, performs bounded synchronous work on first poll, and detaches no
effects. Successful validation may create private `0600` permanent lock
sidecars.

## First-candidate local evidence

The following gates were green on exact first formal candidate `dec98e0`:

- exact Rust 1.94.1 formatting;
- workspace all-target/all-feature Clippy with warnings denied;
- default and all-target/all-feature Rust tests;
- doc tests;
- the 13 focused native session-listing tests;
- 129 repository Python tests with eight expected macOS skips; one
  parallel-load timeout reran green in isolation;
- `cargo deny`;
- `cargo audit --no-fetch` against 1,225 advisories;
- release CLI smoke; and
- diff checks.

These local results do not override the three non-green formal reviews. No
feature-branch or `main` remote CI or benchmark-evidence result is recorded for
this slice.

After first review, isolated production fix `4b8d8b0` implements acquire-first
validation and isolated test hardening `446b495` brings the focused suite to 18
green tests. This replacement candidate composes those results and the corrected
contract. All three replacement tracks are green after the documentation-only
finding corrections at `f0b9fed` and `6689ab9`; executable behavior remains
exactly `3fa54635dab00ebba78b233c69fd39e04e9be57e`.

## First formal review results

### Correctness and API — NOT GREEN

- **Medium:** mandatory independent evidence was missing for per-ID locking,
  concurrent writer behavior, disappearance between enumeration and locked
  read, and unavailable record/lock I/O.
- **Low:** scan and aggregate-byte ceiling documentation omitted the one-entry
  and one-byte overflow witnesses.

### Security and resources — NOT GREEN

- **Medium:** the macOS liveness validation preceded acquisition of the fresh
  `.` enumeration descriptor, leaving a time-of-check/time-of-use window. The
  replacement must acquire first and validate the exact acquired identity.
- **Low:** resource documentation understated the additional fetched/name-
  inspected entry witness and transient byte-transfer witness.

### Documentation, evidence, performance and concurrency — NOT GREEN

- **High:** required independent evidence was missing for raw-cap candidate
  selection, unavailable record and lock I/O, disappearance, a held lock,
  concurrent writer behavior, and synchronous blocking/drop behavior.
- **High:** composition lineage and candidate status were stale, and the
  combined M03 root-and-lifecycle checklist was incorrectly complete before
  formal review and exact remote gates.
- **Medium:** the scan bound omitted the one fetched and name-inspected overflow
  entry.
- **Medium:** the aggregate bound omitted the one transient byte-transfer
  witness used to detect concurrent growth.
- **Medium:** documentation described debug output as universally redacted even
  though the result's derived `Debug` deliberately exposes IDs.
- **Medium:** macOS concurrency wording validated the wrong descriptor order and
  overstated deterministic rename/removal outcomes.

## Replacement rereview results

This replacement composes the acquire-first macOS source fix and the isolated
18-test hardened suite covering the confirmed evidence gaps.

- **Correctness and API — GREEN:** the expanded evidence closes the first-round
  locking, disappearance, writer, unavailable-I/O, and bound-documentation
  findings. The targeted documentation confirmation is green at `6689ab9` with
  behavior unchanged at `3fa5463`.
- **Security and resources — GREEN:** acquire-first validation of the exact
  fresh macOS descriptor closes the liveness window; bounds, lock behavior,
  no-follow access, redaction, and authority were rechecked on `3fa5463`.
- **Documentation, evidence, performance and concurrency — GREEN:** the 18-test
  scope, exact composition status, overflow witnesses, result/error `Debug`
  distinction, macOS concurrency wording, and benchmark/Rust-Zig boundaries
  are consistent at documentation head `6689ab9`, with behavior unchanged at
  `3fa5463`.

## Exact replacement local evidence

Exact Rust/Cargo 1.94.1 formatting, workspace all-target/all-feature Clippy,
default and all-target/all-feature workspace tests, documentation tests, all 18
focused listing tests, and release binary bare/help/status smoke checks are
green. The repository Python suite ran 129 tests with eight expected macOS
skips. `cargo deny check` accepted advisories, bans, licenses, and sources;
`cargo audit --no-fetch` scanned 1,225 cached advisories and 175 lockfile
dependencies with no finding. Diff checks and the worktree are clean.

## Pending delivery evidence

Feature branch head `a70a864673e4df6c7555c06008d43ad802285437` passed exact
benchmark-evidence run `32599591928`, both native macOS jobs, and dependency
policy/audit, but exact CI run `32599591900` failed the existing removed-root
regression in its quality job and on both native Linux architectures. On Linux,
opening fresh `.` from an already removed retained descriptor succeeded and
listing returned an empty success. The prior macOS identity validation did not
run on Linux.

Exact remote-finding behavior candidate
`17f1884c20e84574561eb3cedd96b9aee6d37284` validates the acquired descriptor
on both platforms: Linux rejects a zero link count, while macOS retains its
parent/name identity validation. The acquire-then-unlink unit regression now
compiles and runs on both platforms. Local macOS focused tests, the complete
all-target/all-feature workspace suite, documentation tests, strict workspace
Clippy, and an x86_64 Linux target check are green.

The correctness/API and security/resource/concurrency tracks are green on exact
`17f1884`. The documentation/evidence track found only that this executable
commit could not name its own exact SHA; this documentation-only seal records
that lineage and resolves the finding. Per the user's instruction, the seal
does not receive another adversarial cycle. Replacement exact-SHA remote
workflows remain mandatory.

Exact replacement feature-branch CI and benchmark-evidence workflows, fast-
forward integration, and exact `main` workflows remain pending. Until those
gates pass, the combined root plus native create/list/resume/replay/reset M03
checklist item remains unchecked even though its functional code scope exists.

Milestone 03 also remains in progress because the remaining native tool set,
top-level CLI and slash-command ownership, and composed freshly built release-
binary end-to-end evidence remain open. The machine-god `sessions-json`
benchmark is still unimplemented and claim-ineligible, and Zig remains
benchmark-only.
