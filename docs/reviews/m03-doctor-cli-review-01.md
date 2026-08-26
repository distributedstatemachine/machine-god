# Milestone 03 `doctor` CLI review 01

Status: **IN PROGRESS**

## Base and boundary

- Exact delivered base:
  `f82ce46736f7bac4154da508e3b768d0b9248e15`.
- Feature branch: `agent/m03-doctor-cli`.
- Normative contract: [`doctor-cli.md`](../doctor-cli.md).
- Replacement candidate SHA: **PENDING**. Cycle-1 candidate
  `761bf0bce9faf7b9a40189dc837e38dc2d8e1a82` is rejected.
- Pinned comparison reference: `vercel-labs/fx` commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`.

This ledger covers only strict top-level `doctor [--json]`: exactly four local
`config`, `credential`, `state`, and `platform` checks; exact bounded rendering;
and read-only/no-create behavior. It excludes engine, provider, network,
process, runtime, session, workspace, model, repair, migration, or mutation
behavior. It also excludes compatibility promotion and every product-
performance or fx-equivalence claim.

The slice is not reviewed, workflow-green, integrated, or delivered. The
delivered count remains twenty-nine and Milestone 03 remains in progress.

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

No check is recorded as passed yet. If exact `+1.94.1` is damaged locally, the
only permitted fallback is `+stable` after both Rust and Cargo report release
1.94.1 exactly, and the fallback must be recorded.

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

The composed focused replacement gate is green: native doctor unit and
integration suites pass 10/10 in both no-default and all-feature modes, CLI
doctor unit tests pass 8/8, CLI doctor subprocess tests pass 7/7, formatting is
clean, and focused warnings-denied Clippy passes. The complete replacement gate
and three entirely fresh same-SHA reviews remain pending; cycle-1 reviewers are
not reused.

## Pending evidence

- Native production lineage:
  `667928de751ffff3f1fa58305ae5200b12ca5bdd`, then remediation
  `04444d9c236ff95d52e6c815ed76bb7e0c2a370a`.
- CLI production/evidence lineage:
  `7b540ebe6afd0714603bc12b0945d9986a8ed604`, then partial-write evidence
  `1e5bf28d2a8a29ea5278a177fd7ac75c0b2fc85b`.
- Documentation/evidence component:
  `38ac2f50536692324b5c2b1080acecbe12c20a1a`.
- Independent-evidence lineage:
  `761bf0bce9faf7b9a40189dc837e38dc2d8e1a82`, then remediation
  `3de641f975c235edd19e2fd365f275f85ea2a56b`.
- Fully composed replacement candidate SHA/tree: **PENDING**.
- Cycle-1 complete local gate: **GREEN, CANDIDATE REJECTED BY REVIEW**.
- Replacement focused gate: **GREEN**; complete gate: **PENDING**.
- Formal review cycle 1: **REJECTED, REMEDIATED**.
- Formal replacement review cycle: **PENDING**.
- Exact feature CI and benchmark-evidence workflows: **PENDING**.
- Non-force `main` integration and exact `main` workflows: **PENDING**.

The benchmark schema change is classification evidence only. It records no
sample, comparison result, threshold, performance claim, compatibility
promotion, or fx-equivalence claim.
