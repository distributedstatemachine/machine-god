# Milestone 03 native root-selection review 01

Status: **CANDIDATE — documentation only; production and delivery pending**

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
- Candidate documentation: this isolated commit; exact SHA reported at handoff
- Production implementation: pending
- Independent black-box tests: pending
- Initial composed candidate: pending
- Adversarially green behavior: pending
- Exact feature-gate SHA and workflows: pending
- Documentation seal and workflows: pending
- Exact `main` delivery and workflows: pending
- Final delivery record and workflows: pending
- Integration branch: `agent/m03-native-root-selection`
- Candidate-docs branch: `agent/m03-native-root-selection-docs`
- Toolchain gate: Rust and Cargo 1.94.1 exactly

Thirteen bounded Milestone 03 slices are integrated. This file records the
documentation-only candidate for the fourteenth slice; it is not evidence that
the described production API exists, that independent tests pass, that any
adversarial track is green, or that the slice has passed remote or `main`
delivery gates. Milestone 03 remains `IN PROGRESS`.

## Candidate behavior

The proposed slice adds:

- `NativeRootSelection::from_environment` over an explicitly injected
  `NativeEnvironment` and absolute, parent-component-free workspace path,
  without imposing Unicode on that explicit operating-system path;
- exact nonempty-XDG, empty-XDG fallback, and `HOME` state-root derivation with
  fixed `machine-god` and `.local/state/machine-god` suffixes;
- fixed redacted selection categories for invalid workspace, unavailable state
  input, and invalid selected state environment;
- `PreparedNativeRoots::prepare`, which opens the existing workspace first,
  requires the selected XDG or `HOME` base to exist, and creates only the fixed
  suffix through retained descriptor-relative no-follow operations;
- exact post-open `0700` enforcement for new directories; effective-UID and
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

## Planned parallel delivery and review

Production implementation, independent black-box tests, and candidate
documentation use isolated worktrees with non-overlapping ownership. After
composition, three fresh read-only adversarial tracks will inspect the exact
candidate for correctness/API/tests/portability, security/resources/authority,
and maintained documentation/evidence scope. Every confirmed finding must be
fixed and all three tracks must report green together on the same exact
behavior SHA before feature delivery.

The later documentation-only seal and delivery records will update evidence and
status text only. Per the delivery workflow and the user's explicit instruction,
those documentation-only commits are not adversarially reviewed after
production behavior is already green.

## Candidate documentation checks

The isolated documentation commit is checked with exact Rust/Cargo 1.94.1
workspace doc tests, repository-relative Markdown-link validation, and
`git diff --check`. Exact results are reported at handoff. These checks are not
production, adversarial-review, feature-workflow, benchmark, or `main` evidence.

## Remaining scope

Production, independent tests, composed local gates, all three adversarial
tracks, exact feature workflows, a fast-forward without force to `main`, exact
`main` workflows, and final delivery evidence remain pending. Session lifecycle
and reset/new-incarnation behavior, the remaining native tools, CLI expansion,
release-binary end-to-end host evidence, and compatibility promotion remain
open. No package or GitHub release is authorized.
