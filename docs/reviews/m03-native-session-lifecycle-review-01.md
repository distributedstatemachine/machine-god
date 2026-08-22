# Milestone 03 native session-lifecycle review 01

Status: **RESIDUAL FINDINGS FIXED — FINAL THREE-TRACK REREVIEW PENDING**

## Candidate lineage

- Base and native root-selection final delivery record:
  `d6b1b21c6aab267832ec2043bf6de9c8d1055ee8`
- Isolated production implementation: `021d5b42dadf06906f7c4ce5d317e35023ce7c84`
- Production implementation as composed:
  `60ee848fb388d6e5bdd91043e12fd8f00f2a2a6f`
- Isolated independent contract tests: `a13476e`
- Independent tests as composed:
  `e33046cdc9e2700ae3de8ea82c56074c91858a22`
- Isolated candidate documentation:
  `f1b23a928f7746f9c8cfc45ff8030503d4b31862`
- Candidate documentation as composed:
  `8fc8eeb40279dd19f912c8b2be711af616284db4`
- Allocation-identity portability and test-composition fix:
  `81fb0945bb3decf763d7fd9f64cba3773508ee2f`
- Strict test-lint normalization:
  `d64fe0c6e99f68abadef1fe5df10c3a78795c5f6`
- Initial exact formal-review candidate:
  `d223df75a840e4bed816a56017d2a9501882d285`
- Initial formal API/tests/portability track: **NOT GREEN**; concurrent-create
  error overclaim, corrected with a deterministic regression at `963f21e`
- Initial formal security/resources/authority track: **GREEN**
- Initial formal documentation/evidence track: **NOT GREEN**; five maintained-
  contract findings corrected at `963f21e`
- Formal finding regression and documentation fixes:
  `963f21e9a1c1162599378c3ef2852f16b9d1032f`
- First replacement review candidate:
  `ec17d526a0725ed13a9d523d81cca3882895e969`
- First API/tests/portability rereview on `ec17d52`: **GREEN**
- First security/resources/authority rereview on `ec17d52`: **GREEN**
- First documentation/evidence rereview on `ec17d52`: **NOT GREEN**; one
  engine-authority overclaim plus two low source-contract/evidence findings
- Residual documentation finding fixes:
  `ea9aa89b12a3950414189fbe0ace720f42c64fa4`
- Integration branch: `agent/m03-native-session-lifecycle`
- Toolchain gate: Rust and Cargo 1.94.1 exactly

The production implementation, black-box tests, and candidate documentation
were developed in parallel isolated worktrees with non-overlapping ownership.
Composition exposed one real production defect: `std::ptr::eq` on two
`dyn SessionStore` pointers compared trait-object metadata as well as the data
address, so the exact same `Arc<FileSessionStore>` could be rejected across
codegen units. The fix uses `std::ptr::addr_eq`, preserving the intended exact
allocation-authority check while ignoring non-authoritative vtable identity.
The different-allocation regression remains green.

## Candidate behavior and authority

The composed Rust implementation adds `NativeSessionLifecycle` on Linux and
macOS. It provides durable by-ID create, engine-canonical resume, validated
current-schema durable-record replay, and atomic reset to a new incarnation.
`NativeReferenceHost` retains the lifecycle and the exact concrete file-store
allocation already configured in its engine. A mismatched allocation fails at
construction with fixed redacted `MismatchedSessionStore` before entropy or
filesystem effects.

Production create and reset use a fixed 256-bit operating-system random draw.
The public injected source is a trusted testing or custom-host authority, not
model, configuration, environment, or CLI input. Reset considers at most eight
values and refuses the current incarnation. Create publishes one empty record
at revision `1`. Reset holds the permanent per-ID lock, checks exact observed
ID/incarnation/revision, and atomically replaces the record with the same ID, a
fresh incarnation, revision `old + 1`, next turn sequence `1`, and empty
messages and metadata. It has no deliberate missing-record interval. A stale
old handle is fenced by incarnation on its next save.

All lifecycle futures are inert before first poll. Lifecycle-owned work and the
default source detach nothing; polled filesystem and default-source entropy
work is bounded but synchronous on the polling thread. A trusted custom source
owns its own effects, latency, allocation, detachment, and uniqueness contract.
Present loads may create the fixed permanent lock sidecar. A post-rename
directory-sync failure remains an ambiguous `Unavailable`; callers must
reconcile by resume or replay instead of blindly retrying a mutating operation.
Errors are typed and fixed and retain no session, record, random, path, parser,
or operating-system detail.

The candidate changes no CLI bytes, configuration schema, session file schema,
benchmark workload, compatibility claim, or core authority. It does not add
session listing, session-ID generation, deletion, UI/event replay, or session
commands. Zig remains solely the pinned upstream fx benchmark build input;
machine-god remains a Rust product.

## Independent coverage and local gates

The independently owned focused suite contains thirteen standalone lifecycle
tests and one composed-reference-host test. The initial formal review adds one
deterministic standalone local-create-reservation regression, for fifteen
focused tests total. Coverage includes durable create and
duplicate preservation; exact resume and turn continuation; bounded read-only
replay; reset rotation, clearing, revision monotonicity, stale-handle fencing,
local-live rejection, stale CAS, and prompt/reset races; missing and unpolled
inertness; source failure and exact collision bound; corrupt and exhausted
revision preservation; exact shared-store identity and mismatch rejection;
reference-host wiring; and fixed diagnostic redaction.

The current finding-fix replacement candidate is green under:

- `cargo +1.94.1 fmt --all -- --check`;
- strict locked workspace all-target/all-feature Clippy with `-D warnings`;
- locked default and all-target/all-feature workspace tests;
- workspace documentation tests;
- all fifteen focused lifecycle tests;
- 129 Python tests with eight expected platform skips on macOS;
- `cargo deny check` and `cargo audit --no-fetch` against 1,225 cached
  advisories;
- a freshly built release CLI smoke test; and
- `git diff --check`.

These local checks do not replace exact feature-branch, benchmark-evidence, or
`main` workflow evidence.

## Formal adversarial tracks

Three fresh read-only reviewers inspected exact candidate `d223df75`:

1. public API, independent tests, portability, source compatibility, and
   all-target behavior;
2. security, resources, authority, randomness, store identity, reset
   linearization, cross-process behavior, ambiguity, and redaction; and
3. documentation, evidence, contract consistency, maintained summaries,
   deferred scope, and the Rust-product/Zig-benchmark boundary.

The API track and documentation track both confirmed that the same-engine local
registry reservation can make a concurrent create return `LiveSession` before
it reaches the durable CAS, contradicting an `AlreadyExists`-for-every-loser
documentation promise. Fix `963f21e` distinguishes local registry contention
from a store-CAS loser and adds a deterministic regression that holds the
permanent store lock while two same-engine create attempts overlap.

The documentation track also found that maintained prose conflated standalone
Linux/macOS lifecycle exports with the feature-gated reference host, omitted
the new host getters from its declared public surface, overclaimed that every
local handle blocks reset, and assigned arbitrary trusted custom incarnation
sources the default source's authority and resource guarantees. Fix `963f21e`
states the standalone/host gates separately, lists both getters, describes the
actual incompatible/active/divergent local-state rule, and assigns a custom
source's effects, resource use, latency, detachment, and uniqueness compliance
to its trusted implementor. No production behavior changed for these contract
corrections.

API/tests/portability and security/resources/authority were green on first
replacement `ec17d52`. Documentation/evidence confirmed the original five
findings were materially resolved but found three residual wording defects.
Fix `ea9aa89` now acknowledges that the retained engine transitively owns its
provider, permission, tools, and event sink while lifecycle operations do not
invoke them; labels matching-incarnation reuse as defensive behavior under a
nonconforming source rather than supported coordination; and attributes the
fifteen-test gate to the finding-fix replacement rather than the fourteen-test
initial candidate.

All three tracks must now rereview the same replacement candidate until all
report green. Any confirmed finding changes that candidate and repeats this
requirement. Once
behavior is adversarially green, later documentation-only seal and delivery
records update status and exact workflow evidence only. Per the user's explicit
instruction, those documentation-only commits are not adversarially reviewed.

## Remaining scope

Formal review, exact feature workflows, fast-forward integration without
force, exact `main` workflows, and the final delivery record are pending.
Native `list_sessions` and all lifecycle CLI commands remain open, so the
combined Milestone 03 root-and-lifecycle checklist item stays unchecked. No
package publication or GitHub release is authorized.
