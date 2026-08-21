# Milestone 03 config/status CLI review candidate 01

Status: **REVIEW CANDIDATE — adversarial results pending**.

This is a candidate record, not completion evidence. It must not be relabeled
GREEN until fresh correctness/API, security/abuse, and
performance/resource/evidence reviewers have examined the same integrated
commit and their findings have been resolved. Exact remote CI for that commit
is also pending.

## Candidate scope

- Read-only resolution and metadata inspection for the `machine-god`
  configuration file and state directory, with independent XDG inputs, defined
  `HOME` fallback, explicit invalid/unavailable states, and fixed permission
  mode `ask`.
- Exact bare/help/version/status CLI output, JSON status, argument rejection,
  output-failure behavior, and exit codes documented in [`../cli.md`](../cli.md).
- Bootstrap evidence inventory updates that record help and JSON status as
  implemented but non-equivalent, claim-ineligible, and intentionally
  unmeasured for both fx and machine-god. Doctor, session-list, and background
  workloads remain unimplemented for machine-god and unmeasured for fx.

The native foundation is commit
`806f531954566e2380db5e7529a7e125be653dc0`. This documentation/benchmark
candidate is prepared on top of that commit. The coordinator must record the
exact integrated CLI and review-candidate SHA before review begins; a document
cannot truthfully self-identify the commit that first contains itself.

## Evidence available before adversarial review

The benchmark validator independently fixes the canonical command and state for
all five local-workload records. For help and status it requires both exact
binary commands, `status: not-measured`, `equivalence: non-equivalent`,
`claim_eligible: false`, and no measurement-result fields. For doctor, sessions,
and background it requires canonical fx commands without samples and an
explicit commandless `machine-god` implementation with `status: unimplemented`.
Mutation tests reject changes to equivalence, claim eligibility, status,
commands, samples, aggregate results, and the unimplemented machine shape.

Focused benchmark-schema unit tests pass on this candidate. The complete
repo-wide Python result and documentation checks are recorded in the commit
handoff, not predeclared here. No help or status benchmark was run, no samples
were added, and no compatibility or performance claim was promoted.

## Review focus

Reviewers should challenge:

1. precedence between selected XDG roots and `HOME`, especially empty,
   relative, and non-Unicode values;
2. wrong-kind, inaccessible, missing, and final-symlink classification without
   accidental parsing, canonicalization, creation, or writes;
3. terminal/control injection through human paths, JSON escaping and key order,
   non-UTF-8 arguments, partial/broken output, stderr diagnostics, and exit
   codes; and
4. any evidence-schema path that could time a non-equivalent pair, retain
   samples, or imply that the remaining commands are implemented.

## Deferred scope

This slice does not complete Milestone 03. Configuration parsing or mutation,
permission prompting, executable native tools, concrete providers, durable
native sessions, doctor/session/background commands, fx compatibility, and
product performance claims remain planned.
