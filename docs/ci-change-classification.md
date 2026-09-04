# CI change classification

CI and Benchmark use
[`dorny/paths-filter`](https://github.com/dorny/paths-filter) v4.0.3,
pinned to immutable reference `ceb8a2b8f2d89434be7ff52d3de7ec3738c5cc9d`,
to classify changed paths inside workflows that still start for every
applicable event.
Heavy jobs are conditional; the workflows themselves are not suppressed by
top-level path filters. This leaves stable `CI gate` and `Benchmark gate`
contexts for branch protection.

## Classification boundary

CI classifies independent concerns instead of assigning a single full or
documentation-only label. A documentation concern is selected by changes to
`AGENTS.md`, `README.md`, Markdown below `docs/`, the bounded documentation
checker, or its tests. The four code-coupled documents request focused checks:

- `docs/core-api.md` runs `machine-god-core` doctests;
- `docs/testkit.md` runs `machine-god-testkit` doctests;
- `docs/compatibility.md` verifies offline that the checked-in compatibility
  inventory renders the checked-in Markdown exactly; and
- `docs/vision.md` runs its focused manifest, source, and workflow agreement
  test.

Other Markdown uses only the bounded repository documentation checker and its
policy tests. Mixed changes select both their documentation checks and the
affected code concerns.

Rust routing follows source dependency closure. A core source or manifest
change selects core, testkit, native, and CLI packages. A testkit source or
manifest change selects testkit and its native test consumer. A native source
or manifest change selects native and CLI. A CLI change selects CLI. Changes
confined to a crate's tests, examples, or benchmarks select that crate only. Root Cargo,
lockfile, or toolchain inputs select every package; formatting configuration
selects workspace formatting without package tests. The standalone
reentrant-waker fixture is formatted, linted, and tested through its own
manifest while also selecting its core and native consumers.

Python, compatibility, release-smoke, dependency-audit, native-matrix, and
unsupported-platform concerns are independent. In particular, dependency
policy alone does not run product tests, while shipped core/native/CLI source
selects the release smoke. Compatibility inputs include the policy, inventory,
generator, fixtures, and pinned upstream lock.

An explicit catch-all filter selects every CI concern for any unclassified
non-documentation path, including when an otherwise known path changes in the
same range. Workflow and classifier-test changes also select every concern.
Malformed classifier output defaults to every concern. Classification failure
still fails the aggregate gate.

The compatibility concern separately validates the inventory against the
pinned upstream checkout. Every input to that agreement—lock, policy,
inventory, generator, and workflow—selects that concern, so the focused offline
documentation check does not weaken upstream evidence.

The action runs in local Git mode with an empty API token, receives no
changed-file list output, and interpolates no changed path into a shell
command. Its filters are declared inline in each workflow. The action is
pinned by full commit rather than a mutable version tag.

Git can hide changed gitlinks when effective configuration or `.gitmodules`
configures `diff.ignoreSubmodules` or a submodule `ignore` policy. Both
workflows reject those overrides, malformed configuration, and non-regular
`.gitmodules` paths before classification. The repository therefore keeps
Git's default visibility for tracked submodule changes; policy failure cannot
grant a documentation exemption because the aggregate gate fails.

## Change range

Both checkout and classification bind the head to the event's exact
`github.sha`. On a push, the base is the event's exact `before` SHA, so a push
containing several commits covers the entire pushed range. On a pull request,
the base is the event-local pull-request base SHA and the exact checked-out
merge commit is the head. Classification therefore does not depend on a live
pull-request file-list API or its file-count ceiling. An initial branch push
uses the action's merge-base behavior for the all-zero `before` SHA. Manual
Benchmark dispatch always selects the complete evidence path.

Each workflow result belongs to its own `github.sha`; a later documentation
commit cannot replace the exact-SHA result required for an earlier product
candidate.

## Workflow gates

The CI workflow always runs change classification, documentation policy, and
the final `CI gate`. The quality job receives only fixed package names selected
by the dependency closure; changed paths are never interpolated into commands.
Formatting, Clippy, package tests, and package documentation tests therefore
run only for affected packages. Python, compatibility, and release-smoke steps
have their own selectors. Dependency audit, native target matrices, and
unsupported-platform compilation remain separate conditional jobs. The four
focused documents add only their named checks. The final gate independently
verifies each selector against its job result: selected jobs must succeed and
unselected jobs must be skipped.

The Benchmark workflow always runs classification and the final `Benchmark
gate`. Every non-documentation change and every manual dispatch runs both
artifact-producing benchmark jobs, preserving the repository's exact-SHA
evidence policy. Documentation-only changes skip both and create no benchmark
artifact; the aggregate gate verifies those skips.

A lightweight result is not benchmark evidence for an ancestor. The
implementation plan remains the sole live ledger and retains the last
artifact-producing product evidence until a later product change passes the
full evidence gate. Documentation-only descendants leave that canonical
record unchanged.
