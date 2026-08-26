# Milestone 03 `doctor` CLI review 01

Status: **DELIVERED**

## Base and boundary

- Exact delivered base:
  `f82ce46736f7bac4154da508e3b768d0b9248e15`.
- Feature branch: `agent/m03-doctor-cli`.
- Normative contract: [`doctor-cli.md`](../doctor-cli.md).
- Cycle-3 replacement candidate SHA:
  `15f8176b9322ef989a8c3db01bd404b79d6469fb`, tree
  `8278a777dbfe375e126ce782f581182a73d1e25e`. Cycle-1 candidate
  `761bf0bce9faf7b9a40189dc837e38dc2d8e1a82` and cycle-2 candidate
  `408e6ff54aa53d94965657107da8046945811244` are rejected.
- Pinned comparison reference: `vercel-labs/fx` commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`.

This ledger covers only strict top-level `doctor [--json]`: exactly four local
`config`, `credential`, `state`, and `platform` checks; exact bounded rendering;
and read-only/no-create behavior. It excludes engine, provider, network,
process, runtime, session, workspace, model, repair, migration, or mutation
behavior. It also excludes compatibility promotion and every product-
performance or fx-equivalence claim.

The exact behavior candidate is zero-finding approved and delivered through
the review-exempt seal recorded below. The delivered count is thirty and
Milestone 03 remains in progress.

## Component ownership

Parallel work uses isolated worktrees and non-overlapping files:

1. **Native production** owns authoritative report/check/status/credential-
   status values, process and injected inspection, classification, ordering,
   and counts.
2. **CLI production** owns strict parsing, help/usage, rendering, the 4,096-byte
   complete-output bound, internal four-check snapshot validation, output
   channels, and exit codes.
3. **Independent evidence** owns black-box and injected-boundary tests.
4. **Documentation/evidence** owns the normative contract, maintained
   summaries, benchmark schema/mutation tests, and release-smoke workflow.

No component may revert another agent's work, edit generated compatibility
artifacts, add unrelated behavior, or move product state into the CLI.

## Frozen acceptance boundary

- Argument parsing completes before checks; invalid arguments exit 2.
- A valid invocation reports exactly four ordered checks with closed names,
  statuses, and fixed/redacted detail strings.
- `ok`, `warn`, and `fail` counts are each `0..=4`, sum to four, and match the
  check array. A diagnostic `fail` remains a successful exit-0 report.
- Human and compact JSON reports end in LF and fit the inclusive 4,096-byte cap.
- Render failure is exactly
  `machine-god doctor: could not render report\n`; stdout write failure remains
  `machine-god: failed to write output\n`; both exit 1.
- Inspection is read-only and no-create, with no network, process, runtime,
  session, workspace, model, or path output.
- Linux/macOS report supported; other targets report a platform `fail` while
  retaining the complete report.
- Missing configuration/defaults and missing state are warnings; missing
  generation credential is a failure.
- Bootstrap `doctor-json` is implemented/non-equivalent/not-measured/claim-
  ineligible. Workload order is unchanged, and `sessions-json` plus
  `background-json` remain unimplemented.

## Required local gate

Focused native and CLI tests run first. The exact candidate then must pass:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
python3 -m unittest -v benchmarks.test_benchmarks
```

The compatibility generator `--check`, its tests, documentation integrity, and
a freshly built release-binary matrix are also required. The release smoke uses
isolated missing XDG roots, removes both credential variables, compares human
and JSON check/count content, and proves no root was created.

Cycle-1 and cycle-2 complete gates passed before their candidates were rejected
by review. Exact cycle-3 candidate `15f8176b9322ef989a8c3db01bd404b79d6469fb`
passed the complete gate under exact Rust and Cargo 1.94.1 without fallback. If
exact `+1.94.1` is damaged locally, the only permitted fallback is `+stable`
after both Rust and Cargo report release 1.94.1 exactly, and the fallback must
be recorded.

## Formal review protocol

After the complete local gate, three fresh agents independently inspect the
same immutable candidate SHA and tree:

1. **Correctness/API** — grammar, exact bytes, count invariants, status/detail
   mapping, output cap, exits, and maintained-document agreement.
2. **Configuration/credential/state lifecycle and redaction** — effect order,
   source precedence, read-only/no-create behavior, redaction, platform cfg,
   and absence of authority or sensitive output.
3. **Performance/CLI portability** — bounded work/allocation, error/write
   lifecycle, non-Unicode behavior, dependency delta, cross-target compilation,
   and release-binary checks.

Each report states blocker/high/medium/low counts. Any finding rejects the
candidate, requiring remediation, the complete replacement local gate, and
three fresh same-SHA reviews. Only a zero-finding exact candidate may proceed
to exact feature workflows, non-force fast-forward integration, exact `main`
workflows, and delivery.

## Review cycle 1: rejected and remediated

Exact candidate `761bf0bce9faf7b9a40189dc837e38dc2d8e1a82` passed the
complete local Rust 1.94.1 gate, 135 Python tests with eight host-specific
skips, pinned compatibility regeneration, documentation integrity, dependency
policy, and freshly built release-binary human/JSON/no-create smoke. Installed
no-feature WASI, FreeBSD, and Linux checks passed, as did host all-feature
checks. A local Linux all-feature cross-build reached the host's missing Linux
C sysroot; exact native Linux coverage therefore remains an explicit remote-CI
gate rather than a local success claim.

Three fresh reviewers inspected that same clean SHA and rejected it:

1. Correctness/API reported `blocker=0 high=0 medium=0 low=1`: independent
   evidence omitted configuration `Unreadable`, state `Inaccessible`, and the
   unsupported-platform mapping.
2. Configuration/credential/state lifecycle reported
   `blocker=0 high=0 medium=0 low=1`: the same classification gaps plus no
   direct process-doctor feature-mode evidence.
3. Performance/resource/CLI evidence reported
   `blocker=0 high=0 medium=1 low=1`: Rust argument evaluation captured the
   process credential before configuration inspection, and output failure
   evidence exercised only a writer that rejected its first byte.

The deduplicated findings were remediated in isolated, non-overlapping
worktrees:

- `04444d9c236ff95d52e6c815ed76bb7e0c2a370a` explicitly sequences
  configuration, credential, state, then platform; its mutation test proves
  configuration is inspected before the credential callback, and a pure
  platform classifier exercises both exact branches.
- `3de641f975c235edd19e2fd365f275f85ea2a56b` adds deterministic overlong-path
  `Unreadable`/`Inaccessible` evidence and isolated process-subprocess cases
  for unavailable, missing, API-key, OIDC-precedence, invalid-bearer, and
  non-Unicode credential states without path or secret reflection.
- `1e5bf28d2a8a29ea5278a177fd7ac75c0b2fc85b` proves both human and JSON writes
  that accept a nonempty prefix and then fail still exit 1 with the exact fixed
  output diagnostic.

The composed cycle-2 focused gate was green: native doctor unit and
integration suites pass 10/10 in both no-default and all-feature modes, CLI
doctor unit tests pass 8/8, CLI doctor subprocess tests pass 7/7, formatting is
clean, and focused warnings-denied Clippy passes. Exact candidate
`408e6ff54aa53d94965657107da8046945811244` then passed the complete
replacement gate and proceeded to three entirely fresh same-SHA reviewers;
cycle-1 reviewers were not reused.

## Review cycle 2: rejected and remediated

Exact cycle-2 candidate `408e6ff54aa53d94965657107da8046945811244`
passed the complete Rust 1.94.1 gate, all 135 Python tests with eight
host-specific skips, pinned compatibility regeneration, documentation and
dependency-policy checks, installed WASI/FreeBSD/Linux compilation, and the
fresh release-binary smoke. Three new reviewers inspected that same clean SHA:

1. Correctness/API reported `blocker=0 high=0 medium=0 low=0` and **GREEN**.
2. Configuration/credential/state lifecycle reported
   `blocker=0 high=0 medium=0 low=0` and **GREEN**.
3. Performance/resource/CLI evidence reported
   `blocker=0 high=0 medium=0 low=1`: immediate-error and partial-prefix write
   tests did not mutation-pin a zero-progress writer returning `Ok(0)`, whose
   `WriteZero` handling distinguishes `write_all` from an incorrect one-shot
   `write` implementation.

Any finding rejects a candidate, so the two zero-finding reports do not approve
cycle 2. Isolated remediation
`5a125b6f904f6c84266bbdd6c341963d7c326ddd` adds a zero-progress writer and
exercises both human and JSON commands. Each now proves exit 1, exact fixed
output diagnostic, zero accepted bytes, one write attempt, and one host call.
The composed cycle-3 focused gate is green: formatter, 9/9 doctor CLI unit
tests, and warnings-denied all-target/all-feature CLI Clippy pass under exact
Rust 1.94.1. The complete cycle-3 gate and three entirely fresh same-SHA
reviews are recorded below; neither prior review set was reused.

## Review cycle 3: green

Exact cycle-3 candidate `15f8176b9322ef989a8c3db01bd404b79d6469fb`,
tree `8278a777dbfe375e126ce782f581182a73d1e25e`, passed the four required
workspace commands under exact Rust and Cargo 1.94.1 without fallback. Focused
doctor evidence passed 10/10 native unit and 10/10 native integration tests in
both no-default and all-feature modes, 9/9 CLI unit tests, and 7/7 CLI black-
box tests. The gate also passed 135 Python tests with eight host-specific
skips, pinned compatibility regeneration, documentation integrity across
80 Markdown files, 136 fenced blocks, 574 inline links, and 422 repository-
relative targets with zero errors, dependency policy and advisory checks,
installed WASI/FreeBSD/Linux no-feature compilation, and fresh release-binary
human/JSON/count-consistency/no-create smoke. The release outputs were 254 and
418 bytes and created none of the isolated missing roots.

Three entirely fresh reviewers inspected that same immutable candidate in
separate detached worktrees:

1. Correctness/API reported `blocker=0 high=0 medium=0 low=0` and **GREEN**.
2. Configuration/credential/state lifecycle and portability reported
   `blocker=0 high=0 medium=0 low=0` and **GREEN**.
3. Performance/resource/CLI lifecycle reported
   `blocker=0 high=0 medium=0 low=0` and **GREEN**.

The deduplicated union is zero. The behavior candidate is formally **GREEN**.
The documentation-only review seal containing the locally green record was
exempt from a redundant adversarial cycle; exact feature workflows were the
next gate.

## Delivery evidence

- Native production lineage:
  `667928de751ffff3f1fa58305ae5200b12ca5bdd`, then remediation
  `04444d9c236ff95d52e6c815ed76bb7e0c2a370a`.
- CLI production/evidence lineage:
  `7b540ebe6afd0714603bc12b0945d9986a8ed604`, then partial-write evidence
  `1e5bf28d2a8a29ea5278a177fd7ac75c0b2fc85b`, then zero-progress evidence
  `5a125b6f904f6c84266bbdd6c341963d7c326ddd`.
- Documentation/evidence component:
  `38ac2f50536692324b5c2b1080acecbe12c20a1a`.
- Independent-evidence lineage:
  `761bf0bce9faf7b9a40189dc837e38dc2d8e1a82`, then remediation
  `3de641f975c235edd19e2fd365f275f85ea2a56b`.
- Fully composed cycle-3 replacement candidate SHA/tree:
  `15f8176b9322ef989a8c3db01bd404b79d6469fb` /
  `8278a777dbfe375e126ce782f581182a73d1e25e`.
- Cycle-1 complete local gate: **GREEN, CANDIDATE REJECTED BY REVIEW**.
- Cycle-2 complete local gate: **GREEN, CANDIDATE REJECTED BY REVIEW**.
- Cycle-3 focused and complete gates: **GREEN** under exact Rust 1.94.1.
- Formal review cycle 1: **REJECTED, REMEDIATED**.
- Formal review cycle 2: **REJECTED, REMEDIATED**.
- Formal review cycle 3: **GREEN, 0/0/0/0 IN ALL THREE TRACKS**.
- Documentation-only review seal:
  `345f8125f3ffa029b7ad1df4cf3673428fcf023d`, tree
  `889984990d700712978da933587a437bd13e2a71`.
- Exact feature CI `32933464234` and benchmark-evidence `32933464047`:
  **GREEN**.
- Non-force `main` fast-forward from
  `f82ce46736f7bac4154da508e3b768d0b9248e15`: **COMPLETE**.
- Exact `main` CI `32933879888` and benchmark-evidence `32933879930`:
  **GREEN**.
- Both benchmark runs retain exactly two unexpired exact-SHA artifacts.
- Delivered count: **THIRTY**; Milestone 03: **IN PROGRESS**.

The final delivery-record commit containing this status is documentation-only
and review-exempt. Its own exact feature and `main` workflows are reported at
handoff rather than claimed by this record.

The benchmark schema change is classification evidence only. It records no
sample, comparison result, threshold, performance claim, compatibility
promotion, or fx-equivalence claim.
