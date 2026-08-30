# CI change classification

The CI and Benchmark workflows always start for every pushed commit. A
first-party, standard-library-only classifier decides whether the commit needs
the complete product gate or the documentation-only gate. Heavy jobs are
conditioned inside the workflow; the workflow itself is never suppressed by a
GitHub path filter.

This preserves an exact-SHA result and a stable aggregate gate for branch
protection. It also avoids GitHub path-filter file-count limits and the pending
required-check state that can result when an entire workflow is skipped.

## Documentation-only boundary

A change is documentation-only only when every changed path is one of:

- `README.md`; or
- a Markdown file below `docs/`, except the three exclusions below.

The following Markdown files require the complete product gate:

- `docs/core-api.md` and `docs/testkit.md`, because Rust source includes them
  as crate documentation and their examples are compiled as doctests; and
- `docs/compatibility.md`, because it is generated evidence whose source
  agreement is checked by the pinned-upstream compatibility gate.

`AGENTS.md`, workflow files, scripts, manifests, tests, benchmark inputs,
compatibility inputs, non-Markdown documentation assets, mixed changes, and
every unknown path require the complete gate. Empty or ambiguous diffs also
fail closed to the complete gate.

## Diff selection

For an ordinary push whose nonzero `before` commit exists and is an ancestor
of the new commit, classification uses that exact push range. A new branch uses
the merge base with the repository default branch. Missing objects,
non-ancestor updates, failure to establish the new-branch base, and every other
uncertain push history select the complete gate. Pull requests use the merge
base of the event's base and head commits. Manual dispatch and unknown event
types always select the complete gate.

Git diff is invoked without rename detection and with NUL-delimited output so
both sides of a rename remain visible and unusual path bytes cannot alter the
classification. CI runs are not cancelled by a later commit: a documentation
follow-up cannot erase the separate exact-SHA result for the preceding product
candidate.

## Workflow gates

The CI workflow always runs change classification, the bounded documentation
checker, its focused policy tests, and a final aggregate `CI gate`. Complete
changes additionally run formatting, Clippy, Rust and Python tests, pinned
compatibility drift, release smoke, dependency policy and vulnerability audit,
native target matrices, and unsupported-platform compilation. The aggregate
gate rejects a classifier or documentation failure, a skipped required heavy
job, or a heavy job that ran for a documentation-only change.

The Benchmark workflow always runs classification and a final aggregate
`Benchmark gate`. Complete changes and manual dispatches produce both retained
exact-SHA benchmark artifacts. Documentation-only commits deliberately skip
both expensive evidence jobs and produce no new performance artifact; their
green aggregate result records only that the exemption was correctly applied.
The implementation plan therefore keeps its canonical Benchmark evidence ID
bound to the last delivered behavior commit that produced the artifacts.

Crate-level or tool-level selective Rust testing is deferred. A binary
documentation-versus-complete decision is easier to audit and fails closed as
the repository grows.
