# Milestone 03 Linux containment marker remediation review 01

Status: **ADVERSARIAL GREEN — exact remote rerun pending**

## Candidate and trigger

- Failed sealed parent: `5bf772fa271c348ff0b3d36cac81c5240c6645c1`
- Adversarially green remediation: `4bd039750d792e3c30cbc4d3c1909c644edfb673`
- Branch: `agent/m03-read-file-reviewed`
- Toolchain: Rust and Cargo 1.94.1 exactly

The sealed `read_file` parent passed its exact
[benchmark-evidence workflow](https://github.com/distributedstatemachine/machine-god/actions/runs/32506610549).
Its exact [CI workflow](https://github.com/distributedstatemachine/machine-god/actions/runs/32506610919)
failed at the 129-test Python step; the compatibility-inventory and
release-smoke steps later in that job were consequently skipped. The
final-sample executable-replacement regression expected an identity-change
diagnostic but instead received `invalid literal for int() with base 10: ''`.
Formatting, Clippy, Rust tests, documentation tests, and native tests on Linux
x86_64, Linux arm64, macOS x86_64, and macOS arm64 were green. No `main`
mutation followed that failed gate.

## Root cause and resolution

The collector regression deliberately replaced its model binary after the
eleventh pinned execution. It did not isolate repository provenance, so the
collector called real Git before its final source-identity check. On Linux that
Git subprocess triggered the benchmark harness's one-time hostile-descendant
containment preflight.

The preflight grandchild previously used `Path.write_text` directly on the
final PID marker. The parent treated path existence as publication completion.
Creation can become visible before the short PID payload is written, so a
scheduler interleaving could expose an empty marker to `int(...)`.

The remediation:

- writes the complete ASCII decimal PID to `hostile.pid.partial` in the same
  fresh private temporary directory;
- closes that staging write before `os.replace` atomically publishes the final
  `hostile.pid` name;
- keeps pre-publication failure on the existing bounded, fail-closed timeout and
  cleanup path;
- mocks `repository_head` in the executable-replacement regression so that test
  covers pinned execution and the intended final source-identity rejection,
  while separate tests retain real Git and containment coverage; and
- uses equal-length, equally executable original and replacement fixtures, so
  rejection cannot pass merely because file size or mode changed.

The behavior change, regression update, and matching performance-harness guide
are one conventional commit. It changes no Rust, Cargo, workflow, CLI,
compatibility inventory, pinned fx revision, Zig setup, or product behavior and
makes no performance claim.

## Adversarial review

Three fresh read-only reviewers examined exact remediation SHA
`4bd039750d792e3c30cbc4d3c1909c644edfb673` after local gates:

- correctness and test semantics: **GREEN**;
- concurrency, portability, failure paths, and cleanup boundedness: **GREEN**;
- scope, provenance, documentation, and claim discipline: **GREEN**.

They confirmed that same-directory replacement exposes either no final marker
or the complete PID, never the staging bytes; scheduler delay can only cause a
controlled timeout; temporary-directory cleanup covers either name; and no
crash-durability promise is implied or required. The focused replacement
regression also passed 100 consecutive executions during adversarial review.

One pre-seal recommendation was accepted: change the replacement fixture from
a different-size payload to a distinct payload with the same 21-byte length as
the original. Another recommendation was accepted before review: do not assert
that provenance must occur exactly once, because a future valid earlier
identity check could reject the replacement before provenance collection.

## Exact local checks

The following passed on the exact remediation SHA:

- exact Rust and Cargo 1.94.1 identity checks;
- `cargo fmt --all -- --check`;
- workspace/all-target/all-feature Clippy with warnings denied;
- workspace tests: 234 top-level tests plus 15 deep-JSON subprocess probes;
- workspace documentation tests: 2;
- focused final-sample replacement regression;
- repo-wide Python suite: 129 run, 121 passed, and 8 expected macOS platform
  skips;
- pinned compatibility inventory against fx commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- exact release build and bare/help/status JSON smoke checks;
- `cargo-deny` 0.20.2: advisories, bans, licenses, and sources accepted;
- `cargo audit` 0.22.2 with `--no-fetch`: 1,225 cached advisories checked across
  39 lockfile dependencies with no finding;
- 44 relative documentation links; and
- Python compilation, `git diff --check`, and a clean worktree.

The local host is macOS, so Linux-only containment tests remain an exact remote
CI gate. The reviewed remediation and its documentation-only evidence seal must
pass both feature-branch workflows before any fast-forward of `main`; the same
exact integrated SHA must then pass both workflows on `main`.
