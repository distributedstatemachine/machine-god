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

## Review round 02 finding and remediation

Fresh correctness/API and security/abuse review of exact commit
`b23b293a020b39a5c6a89f59537dad4efdcb4b95` agreed on one further
evidence-integrity gap. Exact keys stopped at the root, workload, and top-level
measurement objects. Nested source, host, tool, command, build, binary,
executable-identity, pinned-executable, and sample objects could still carry
undeclared claim-like or measurement-like fields. Validation now fixes the key
set at every schema-2 object level, while environment maps remain governed by
their existing exact allowlists. A representative mutation suite injects
claims, winners, comparisons, results, and aggregates into every nested object
kind and requires rejection.

All reviewed native and CLI behavior, including both output formats' bidi
escaping, was GREEN in this round. The evidence remediation still requires a
fresh exact-SHA three-way review and complete local and remote gates.

## Review round 03 finding and remediation

Correctness/API and performance/resource/evidence review of exact commit
`48cb452aa0a912c5d61a4ecf04fcb714253c177e` was GREEN. Their generic probes
injected undeclared fields into all 74 object instances and claim prose into all
22 narrative fields; every mutation was rejected. The independent
security/abuse pass then found two final malformed-input edges:

1. With an expected upstream lock, a non-object `tools` value was dereferenced
   before the validator reached its structural check, producing an internal
   `AttributeError` instead of a controlled validation error. Lock comparison
   is now split so repository/commit validation precedes tool parsing and the
   Zig binding occurs only after exact tool validation. The command-line
   checker also rejects a non-object JSON root before calling `.get`.
2. Rust and Cargo versions used prefix checks, so arbitrary claim-bearing text
   could be appended to an otherwise valid version. Git version was likewise
   only nonempty. Exact-toolchain Rust/Cargo outputs and the supported Git
   output forms now use full-string patterns; Zig was already exact. Mutation
   tests require malformed tool structures and appended claim prose to fail as
   `ValueError`.

These fixes require another fresh exact-SHA review and all local/remote gates.

## Deferred scope

This slice does not complete Milestone 03. Configuration parsing or mutation,
permission prompting, executable native tools, concrete providers, durable
native sessions, doctor/session/background commands, fx compatibility, and
product performance claims remain planned.
