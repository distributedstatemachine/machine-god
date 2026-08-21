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

## Review round 04 finding and remediation

Correctness/API review of exact commit
`26764691605fff387cd1919f7fe9afc7d608f6e7` was GREEN. The independent
malformed-input review found three scalar type paths that still raised
`TypeError` rather than the validator's controlled `ValueError`: an unhashable
manifest mode, an unhashable pinned-executable method, and a non-path build
working directory used before command-record validation. The two membership
checks now require strings, and build path comparison occurs only after the
record's exact type validation. A property-style regression replaces every
scalar leaf in a valid evidence tree with an object and requires `ValueError`.
The command-line checker additionally converts invalid JSON, decoding failures,
and defense-in-depth `TypeError` results into one-line failures without Python
tracebacks.

The remediation requires fresh exact-SHA adversarial review and complete local
and remote gates.

## Review round 05 finding and remediation

Correctness/API review of exact commit
`0cd66a7d3d81ab291378e12f7a98b380a4d4739f` was GREEN. The independent
performance/resource/evidence review ran 3,238 cross-type mutations and found
one Python-specific evidence bypass: ordinary dictionary equality treated JSON
integer `0` as equal to `false` and integer `1` as equal to `true` in the
environment-policy record. Validation now requires that record's exact keys
and checks its values with boolean identity. Direct regressions cover both
integer lookalikes, and a property-style suite replaces every scalar leaf with
a different JSON type, including boolean-to-integer substitutions, and requires
controlled rejection.

The remediation requires a fresh exact-SHA three-way review and all local and
remote gates.

## Review round 06 findings and remediation

Correctness/API and performance/resource/evidence reviewers examined exact
commit `5e71c868a975667fce343437277fdb3122974d07` and found two independent
serialization-type gaps:

1. Command and measurement timeout equality accepted JSON `true` when the
   declared timeout was numeric `1`/`1.0`; measurement aggregates could
   similarly accept `true` when the derived median or p95 was `1`. Timeout
   comparisons now require a positive non-boolean number first, and aggregate
   comparisons require a non-boolean integer. Direct aggregate-one regressions
   and an exhaustive alternate-JSON-type sweep cover the equality collisions.
2. Python's ordinary JSON decoder silently retained only the last occurrence
   of a duplicate object name. Raw evidence could therefore contain a false
   claim or sample field followed by the canonical field and appear valid after
   decoding. The checker now uses an object-pairs hook that rejects duplicate
   names at every depth and a constant hook that rejects non-standard `NaN` and
   infinities. Root and nested duplicate/non-finite cases require a controlled
   one-line error without a traceback.

The remediation requires fresh exact-SHA review and complete local and remote
gates.

## Review round 07 findings and remediation

Correctness/API and performance/resource/evidence review of exact commit
`9a47865e7a4e68ba9918a4599dbd1d0943af5b5c` found two remaining robustness
classes:

1. Converting an arbitrary-size JSON integer through `math.isfinite` or
   `statistics.median` could raise `OverflowError`; exponent notation such as
   `1e9999` could also decode to infinity without invoking the decoder's
   constant hook. Integer positivity now avoids float conversion, both evidence
   schemas derive medians with exact integer arithmetic, and JSON float parsing
   rejects non-finite results. Regressions cover 4,000-digit timeouts and
   samples plus exponent overflow without tracebacks.
2. Legacy schema-1 bootstrap validation still allowed undeclared root/host/
   binary fields, extra command arguments, and non-object binary metadata that
   could cause an attribute error. Schema 1 now fixes its root, host, binary,
   and single-executable command shapes before field access. Mutations inject
   claims, results, malformed binaries, and extra arguments and require
   controlled rejection.

The remediations require fresh exact-SHA review and all local and remote gates.

## Review round 08 findings and remediation

Two fresh review passes examined exact commit
`16c7d5aa45d063fcb54cbb8dbf6ad59ba96ffdcf`. The native status and CLI
contract remained GREEN, and the schema-2 validator rejected all malformed
container and alternate-scalar mutations exercised by the reviewers. Two
schema-1 collector/checker integration gaps remained:

1. The bootstrap collector calculated an even-length median through
   floating-point `statistics.median`, while the checker had already moved to
   exact integer arithmetic. Large integer samples could therefore produce an
   artifact that the checker rejected. The collector now uses the same exact
   integer floor-median rule, and a collector-to-checker regression covers
   arbitrarily large samples whose middle values differ.
2. Embedded-NUL evidence paths reached `Path.resolve()` outside a controlled
   error boundary, and a missing supplied binary could similarly let a
   filesystem exception escape. Evidence-path resolution and supplied-binary
   inspection now convert filesystem/path failures into one-line validation
   errors without tracebacks. Regressions cover both NUL-bearing recorded paths
   and a missing supplied binary.

The remediations require another fresh exact-SHA three-way review and all local
and remote gates.

## Review round 09 finding and remediation

Fresh correctness/API and performance/resource/evidence reviewers independently
examined exact commit `777c7a4b1594f52c000213ba01820244e6bc1a84` and
found one remaining schema-1 supplied-binary validation flaw. The checker
hashed a path before rejecting its size and did not first require an executable
regular file. A character device such as `/dev/zero` could therefore make the
checker read indefinitely, while a matching non-executable regular file was
accepted despite the collector's executable-binary requirement.

The checker now opens the supplied path once with nonblocking, close-on-exec,
and no-follow flags where the platform provides them. Descriptor metadata must
identify a regular executable of the declared size before hashing. Hashing is
bounded to that declared size plus one end-of-file probe, so concurrent growth
cannot turn validation into an unbounded read. Regressions cover non-regular
paths, non-executable regular files, and the exact read bound.

This remediation requires another fresh exact-SHA three-way review and all
local and remote gates.

## Review round 10 finding and remediation

Fresh correctness/API and performance/resource/evidence reviewers independently
examined exact commit `03ecbc95a853c5f2e145d49bc15b3e7033e4bec7`.
The schema-1 supplied-binary remediation was GREEN, but the schema-2 validator
still performed separate pathname metadata checks followed by an unbounded
reopen-and-hash operation. A same-size replacement could disconnect the file
whose executable status was checked from the bytes later hashed, and replacing
the path with a device could make validation read indefinitely.

Schema-2 binary validation now opens one nonblocking, close-on-exec, no-follow
descriptor where those flags are available. It checks regular-file and
executable status and declared size on that descriptor, hashes only the
declared size plus one end-of-file probe, and verifies both the descriptor and
canonical pathname identities after hashing. All paths close the descriptor.
Regressions cover non-regular and non-executable files, bounded growth,
failure-path cleanup, and deterministic pathname replacement.

This remediation requires another fresh exact-SHA three-way review and all
local and remote gates.

## Review round 11 findings and remediation

Fresh correctness/API and performance/resource/evidence reviewers examined
exact commit `f25ff2a4c1e31070edc383eae061865733cf21f5`. The schema-2
descriptor binding passed targeted device, FIFO, growth, deletion, replacement,
and descriptor-leak probes. Two lower-severity evidence-validation gaps
remained:

1. `generated_at_utc` accepted any nonempty string, including invalid dates,
   offset variants, and claim-bearing prose. Validation now parses and
   round-trips exactly the UTC `Z` representation emitted by the collector.
   Boundary and mutation tests cover valid canonical timestamps, invalid dates,
   offsets, alternate fractional precision, and prose.
2. A `RuntimeError` from post-validation executable-identity inspection could
   escape the validator and produce a checker traceback. Both `OSError` and
   `RuntimeError` are now converted at that boundary into a cause-free
   `ValueError`. Direct and checker-process regressions require controlled
   one-line failure.

These remediations require another fresh exact-SHA three-way review and all
local and remote gates.

## Review round 12 finding and remediation

Three reviewers examined exact commit
`06c4893918905a342545cc28bb80f1519d3f8136`. Correctness/API and
performance/resource/evidence passes were GREEN, including 107 repo-wide
Python tests, native/CLI checks, formatting, Clippy, release smoke, no-write
behavior, timestamp mutations, raw-JSON failures, and binary device, growth,
replacement, and descriptor-leak probes. The independent final reviewer found
one shared resource-bound gap: executable identity still hashed a regular tool
or measured binary until end-of-file, so concurrent growth could prevent
validation or collection from terminating.

Executable identity now opens the canonical target with nonblocking,
close-on-exec, and no-follow flags where available; binds target metadata to the
descriptor; hashes the initial descriptor size plus one end-of-file probe; and
then revalidates the invocation entry, canonical target pathname, and descriptor
metadata. Symlink invocation support is retained. Regressions cover bounded
growth and read count, descriptor cleanup, canonical-target replacement behind
an unchanged symlink, and same-path content mutation.

This remediation requires another fresh exact-SHA three-way review and all
local and remote gates.

## Review round 13 findings and remediation

Three reviewers examined exact commit
`803f0d3f7601da522bf49c464f4bc5aa398cc179` and found two related
identity/resource gaps:

1. Rechecking only the top invocation symlink and original target missed a
   retargeted intermediate symlink in a multi-hop chain. Executable identity now
   resolves the invocation again after hashing and requires the same canonical
   target. A two-hop retarget regression covers the case.
2. Binary record collection, descriptor hashing, and the pinning copy still had
   end-of-file-driven reads after identity capture. Concurrent growth could
   therefore keep collection or measurement preparation running indefinitely.
   Descriptor hashing and copying now consume exactly the recorded size plus
   one end-of-file probe, recheck descriptor and canonical-path metadata, and
   retain guaranteed cleanup. `binary_record` uses the bounded executable
   identity path, and the general file hasher now uses one descriptor with the
   same bounded-read and post-read identity rules.

Regressions cover exact descriptor hash/copy read counts with an always-readable
source, growing executable and generic files, intermediate-link retargeting,
and existing pinning behavior. These remediations require another fresh
exact-SHA three-way review and all local and remote gates.

## Review round 14 findings and remediation

Three reviewers examined exact commit
`b7c1a2438d1093acb121d1851127de32ee6316bb`. They independently found
that a completed pinned executable leaked its descriptor if final source
identity verification failed after construction. The evidence/resource pass
also identified the two remaining production end-of-file reads: schema-1
binary metadata hashing and materialized-source file collection.

Pinned-resource ownership now remains under one cleanup guard through final
source verification; failures close the source, pinned descriptor, and private
temporary directory without masking the active error. Adjacent measurement
cleanup preserves its active error as well. Schema-1 binary metadata and
materialized-source files are now collected through nonblocking, no-follow
descriptors with initial-size-plus-one-byte bounds and pre/post pathname and
descriptor identity checks. The production read audit found no other
end-of-file file reads; remaining raw reads are fixed one-byte process
synchronization operations.

Regressions cover final-verification descriptor and temporary-directory
cleanup, schema-1 binary growth and replacement, and materialized-source growth
and replacement. These remediations require another fresh exact-SHA three-way
review and all local and remote gates.

## Deferred scope

This slice does not complete Milestone 03. Configuration parsing or mutation,
permission prompting, executable native tools, concrete providers, durable
native sessions, doctor/session/background commands, fx compatibility, and
product performance claims remain planned.
