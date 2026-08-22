# Milestone 03 native root-selection review 01

Status: **COMPOSED CANDIDATE — formal adversarial and delivery gates pending**

## Candidate lineage

- Base and configured credential-source final delivery record:
  `f840576af241c58d1e55399e66ba92f7770cd50c`
- Exact base feature CI run:
  [`32583585145`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583585145)
- Exact base feature benchmark-evidence run:
  [`32583585148`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583585148)
- Exact base `main` CI run:
  [`32583871385`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583871385)
- Exact base `main` benchmark-evidence run:
  [`32583871368`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583871368)
- Isolated candidate documentation:
  `9ee2bbf3300fbbbdb7d9c13d5610c1b01da44522`
- Candidate documentation as composed:
  `41f177c6a35e1b456dff39040a2c536ed5a0f9f8`
- Production implementation as composed:
  `050d253e9cdd901996883c7130508488f9aabdbb`
- Created-directory normalization fix:
  `7420a3a2664144900bb2eb688c601d9f41480e53`
- Selected-state-base normalization fix:
  `fa5119a91102159392e639ecba473cb01495af44`
- Independent two-test root-preparation regression suite:
  `85c99a86889e38c3a8671a30ee320224744bed2d`
- Independent nine-test root-selection core-contract suite:
  `85c4193cbf7efabb7916a2a75d3a9e0e3bae3440`
- Independent three-test all-feature prepared-host suite:
  `236e3d45f71074d6f3fe2817958f8b89bf9a3ac9`
- Preliminary read-only audit: **GREEN**; explicitly not one of the required
  three fresh formal adversarial tracks
- Initial composed candidate: this documentation-reconciliation commit; exact
  SHA reported at handoff
- Adversarially green behavior: pending
- Exact feature-gate SHA and workflows: pending
- Documentation seal and workflows: pending
- Exact `main` delivery and workflows: pending
- Final delivery record and workflows: pending
- Integration branch: `agent/m03-native-root-selection`
- Candidate-docs branch: `agent/m03-native-root-selection-docs`
- Toolchain gate: Rust and Cargo 1.94.1 exactly

Thirteen bounded Milestone 03 slices are integrated. The fourteenth slice's
production, two correctness fixes, and 14 independently owned focused tests are
now composed, and focused root-selection and prepared-host gates are green. A
preliminary read-only audit is green but is not formal review evidence. No
required fresh adversarial track is yet green on this exact composed candidate,
and feature, `main`, and final-delivery gates remain pending. Milestone 03
remains `IN PROGRESS`.

## Candidate behavior

The composed candidate implements:

- `NativeRootSelection::from_environment` over an explicitly injected
  `NativeEnvironment` and absolute, parent-component-free workspace path,
  without imposing Unicode on that explicit operating-system path;
- exact nonempty-XDG, empty-XDG fallback, and `HOME` state-root derivation with
  lexical-component rebuilding of the accepted base and fixed `machine-god`
  and `.local/state/machine-god` suffixes;
- fixed redacted selection categories for invalid workspace, unavailable state
  input, and invalid selected state environment;
- `PreparedNativeRoots::prepare`, which opens the existing workspace first,
  requires the selected XDG or `HOME` base to exist, and creates only the fixed
  suffix through retained descriptor-relative no-follow operations;
- descriptor-relative pre-open normalization followed by post-open identity and
  exact `0700` enforcement for new directories; effective-UID and
  no-group/other-write validation for the selected base and existing
  intermediates; complete group/other privacy for the existing final root; and
  validation rather than chmod or repair for any existing directory;
- retained workspace and state-root descriptors and mandatory device/inode plus
  descriptor-parent-walk rejection when their opened identities are equal or
  one is an ancestor of the other;
- fixed redacted preparation categories for workspace, state-base, state-root,
  unsafe-directory, and overlapping-root failure;
- production and trusted-custom-transport reference-host constructors that
  consume prepared roots without reopening their paths and keep credential
  discovery after retained-root validation; and
- unchanged no-create behavior for the existing reference-host path
  constructors and `FileSessionStore::open`.

The normative candidate contract is
[`native-root-selection.md`](../native-root-selection.md).

## Authority and scope

Selection consumes injected data only and performs no filesystem effect.
Preparation owns narrowly bounded root authority on Linux and macOS: it may
open the explicit workspace and selected state base and may create only the
constant suffix directories. It cannot select an arbitrary state suffix,
create the selected base, chmod or chown an existing directory, clean up
arbitrary state, or delegate environment or creation authority to core or the
CLI. Its error and debug surfaces reflect no path, environment value,
ownership/mode detail, descriptor identity, operating-system diagnostic, or raw
error number.

Schema v3, built-in and file configuration bytes, status inspection, and every
CLI output/error byte remain unchanged. Status remains metadata-only and
no-create. No session create/list/resume/replay/reset behavior or reset
incarnation allocation is included. The combined root-and-lifecycle checklist
item therefore remains unchecked.

This slice adds no compatibility, upstream-equivalence, or product-performance
claim. Zig remains solely the pinned upstream benchmark build input;
machine-god remains a Rust product.

## Parallel delivery and pending formal review

Production implementation, independent black-box tests, and candidate
documentation were completed in isolated worktrees with non-overlapping
ownership, then composed with both correctness fixes. The focused regression,
core-contract, and prepared-host suites run 2, 9, and 3 tests respectively and
are green. A preliminary read-only audit reported green, but it is explicitly
not one of the required formal tracks.

Three fresh read-only adversarial tracks must still inspect the exact composed
candidate for correctness/API/tests/portability, security/resources/authority,
and maintained documentation/evidence scope. Every confirmed finding must be
fixed and all three tracks must report green together on the same exact
behavior SHA before feature delivery.

The later documentation-only seal and delivery records will update evidence and
status text only. Per the delivery workflow and the user's explicit instruction,
those documentation-only commits are not adversarially reviewed after
production behavior is already green.

## Candidate documentation checks

This composed documentation reconciliation passes:

- exact Rust/Cargo 1.94.1 workspace doc tests: two passed, zero failed;
- `cargo +1.94.1 fmt --all -- --check`;
- 169 repository-relative links across 48 Markdown files with none missing; and
- `git diff --check`.

These checks and the focused production tests are not formal adversarial-
review, feature-workflow, benchmark, or `main` evidence.

## Remaining scope

Three fresh formal adversarial tracks, any resulting fixes and rereviews, full
local gates, exact feature workflows, a fast-forward without force to `main`,
exact `main` workflows, and final delivery evidence remain pending. Session
lifecycle and reset/new-incarnation behavior, the remaining native tools, CLI
expansion, release-binary end-to-end host evidence, and compatibility promotion
remain open. No package or GitHub release is authorized.
