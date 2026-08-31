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

A change is documentation-only when every changed path is either `README.md`
or Markdown below `docs/`. The four code-coupled documents remain
documentation-only but request focused checks:

- `docs/core-api.md` runs `machine-god-core` doctests;
- `docs/testkit.md` runs `machine-god-testkit` doctests;
- `docs/compatibility.md` verifies offline that the checked-in compatibility
  inventory renders the checked-in Markdown exactly; and
- `docs/vision.md` runs its focused manifest, source, and workflow agreement
  test.

Other Markdown uses only the bounded repository documentation checker and its
policy tests. A change to any other path, including a mixed documentation and
code change, selects the complete product gate. Classifier failure does not
grant a documentation exemption: the aggregate gates fail.

The complete product gate separately validates the inventory against the
pinned upstream checkout. Every input to that agreement—lock, policy,
inventory, generator, and workflow—remains a full-gate path, so the focused
offline check does not weaken upstream evidence.

The action runs in local Git mode with an empty API token, receives no
changed-file list output, and interpolates no changed path into a shell
command. Its filters are declared inline in each workflow. The action is
pinned by full commit rather than a mutable version tag.

Git can hide changed gitlinks when `.gitmodules` configures a submodule
`ignore` policy. Both workflows reject any such override, and reject malformed
`.gitmodules` configuration, before classification. The repository therefore
keeps Git's default visibility for tracked submodule changes; policy failure
cannot grant a documentation exemption because the aggregate gate fails.

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
the final `CI gate`. Documentation-only changes skip formatting, Clippy, Rust
workspace tests, release smoke, dependency audit, native target matrices, and
unsupported-platform compilation. The four focused documents add only their
named checks. Complete and mixed changes run the full product jobs. The final
gate verifies that classification and documentation succeeded and that heavy
jobs either all succeeded or all skipped as required.

The Benchmark workflow always runs classification and the final `Benchmark
gate`. Complete changes and manual dispatches run both artifact-producing
benchmark jobs. Documentation-only changes skip both and create no benchmark
artifact; the aggregate gate verifies those skips.

A lightweight result is not benchmark evidence for an ancestor. The
implementation plan remains the sole live ledger and retains the last
artifact-producing product evidence until a later product change passes the
full evidence gate. Documentation-only descendants leave that canonical
record unchanged.
