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

## Review round 01 findings and remediation

The correctness/API and security/abuse reviewers examined exact commit
`141fea29df0cafff1e50cf8de161ec22b593a4eb` and independently confirmed two
findings. The performance/resource/evidence review of that same immutable
commit continued in parallel.

1. JSON syntax escaping covered C0/C1 controls and line separators but left
   Unicode bidirectional-formatting controls literal. A hostile path could
   visually reorder terminal status despite remaining valid JSON. The encoder
   now also escapes `U+061C`, `U+200E`–`U+200F`, `U+202A`–`U+202E`, and
   `U+2066`–`U+2069`. Unit and process-level regressions cover both human and
   JSON status and assert that none of those raw controls reach stdout.
2. Implementation records for unmeasured workloads had exact-key checks, but
   the evidence root and workload objects accepted undeclared fields. An
   artifact could therefore attach claim-like or measurement-like material
   outside the checked implementation records. Validation now requires the
   exact schema-2 root, workload, and measured-bootstrap implementation key
   sets. Mutation tests inject false performance claims, comparison objects,
   availability flags, winners, results, samples, and aggregates at those
   levels and require rejection.

These remediations are not a GREEN result by themselves. They require fresh
review of their exact committed SHA, followed by the complete local and remote
gates.

The performance/resource/evidence reviewer then identified a third finding on
the same exact round-01 commit:

3. Workload and implementation key sets were fixed, but their nonempty
   `description` and `reason` strings remained free-form. An evidence artifact
   could replace them with prose asserting false equivalence or invented
   measurements while retaining structurally claim-ineligible fields. The
   bootstrap narrative now uses shared constants in collection and validation,
   and the five local workload records are validated against the complete
   canonical structures produced by their shared definition. Mutation tests
   replace bootstrap, implemented, unimplemented, fx, and machine-god prose
   with claim-bearing statements and require rejection.

## Deferred scope

This slice does not complete Milestone 03. Configuration parsing or mutation,
permission prompting, executable native tools, concrete providers, durable
native sessions, doctor/session/background commands, fx compatibility, and
product performance claims remain planned.
