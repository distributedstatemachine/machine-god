# Milestone 03 `doctor` CLI review 01

Status: **IN PROGRESS**

## Base and boundary

- Exact delivered base:
  `f82ce46736f7bac4154da508e3b768d0b9248e15`.
- Feature branch: `agent/m03-doctor-cli`.
- Normative contract: [`doctor-cli.md`](../doctor-cli.md).
- Exact candidate SHA: **PENDING**.
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
2. **Configuration/credential/state security** — effect order, source
   precedence, read-only/no-create behavior, redaction, platform cfg, and
   absence of authority or sensitive output.
3. **Performance/CLI portability** — bounded work/allocation, error/write
   lifecycle, non-Unicode behavior, dependency delta, cross-target compilation,
   and release-binary checks.

Each report states blocker/high/medium/low counts. Any finding rejects the
candidate, requiring remediation, the complete replacement local gate, and
three fresh same-SHA reviews. Only a zero-finding exact candidate may proceed
to exact feature workflows, non-force fast-forward integration, exact `main`
workflows, and delivery.

## Pending evidence

- Production component SHA/tree: **PENDING**.
- CLI component SHA/tree: **PENDING**.
- Independent-evidence component SHA/tree: **PENDING**.
- Fully composed candidate SHA/tree: **PENDING**.
- Focused and complete local gates: **PENDING**.
- Formal review cycle 1: **PENDING**.
- Exact feature CI and benchmark-evidence workflows: **PENDING**.
- Non-force `main` integration and exact `main` workflows: **PENDING**.

The benchmark schema change is classification evidence only. It records no
sample, comparison result, threshold, performance claim, compatibility
promotion, or fx-equivalence claim.
