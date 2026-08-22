# Milestone 03 native root-selection review 01

Status: **FINDING FIXES COMPOSED — exact-SHA formal rereview and delivery gates pending**

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
- Initial exact formal-review candidate:
  `d59c7a5b19def6bf4c65806efa3bf3986d77cb7d`
- Formal portability finding fix for valid fixtures under restrictive ambient
  umasks: `f5dbbca`
- Formal security finding fix for descriptor-bound macOS extended ACL
  rejection: `8ae17db`
- Independent macOS ACL regression: `041c83c`
- Formal compatibility finding regression for protective macOS deny-delete
  ACLs: `bb2a856`
- Protective macOS ACL policy and public Rustdoc finding fix: `fa94d8a`
- Preliminary read-only audit: **GREEN**; explicitly not one of the required
  three fresh formal adversarial tracks
- Initial formal documentation track on `d59c7a5`: **GREEN**
- Initial formal API/tests/portability track on `d59c7a5`: **NOT GREEN**;
  one medium fixture-mode finding, fixed at `f5dbbca`
- Initial formal security/resources/authority track on `d59c7a5`: **NOT
  GREEN**; one high macOS ACL finding, fixed at `8ae17db` and covered at
  `041c83c`
- First exact finding-fix behavior candidate:
  `107d43490ac3babdc32e55b3bb5b24ab2ef17329`
- First API/tests/portability rereview on `107d434`: **NOT GREEN**; one low
  public Rustdoc mismatch, fixed at `fa94d8a`
- First security/resources/authority rereview on `107d434`: **NOT GREEN**;
  one medium ordinary-macOS-HOME compatibility finding, covered at `bb2a856`
  and fixed at `fa94d8a`
- First documentation/evidence rereview on `107d434`: **NOT GREEN**; the same
  medium HOME-ACL compatibility finding plus one medium stale README/reference-
  host evidence finding, fixed in the next finding-fix candidate
- Next exact finding-fix behavior candidate: this reconciliation commit; exact
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
production, finding fixes, and 16 independently owned focused tests are now
composed, and focused root-selection and prepared-host gates are green. The
initial formal review produced one confirmed portability finding plus one
confirmed security finding. First rereview produced a Rustdoc mismatch, a
protective macOS HOME-ACL compatibility issue, and stale summary evidence. All
are fixed in this candidate. No required formal track is yet green on the same
exact finding-fix behavior SHA,
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
- descriptor-bound acceptance on macOS of only an empty ACL or exact flag-free
  deny-delete entries with no ACL-level flags, rejecting every other entry,
  malformed value, or ACL-read failure for the selected base and every retained
  existing or newly created suffix directory;
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

## Parallel delivery and formal finding resolution

Production implementation, independent black-box tests, and candidate
documentation were completed in isolated worktrees with non-overlapping
ownership, then composed with the correctness and formal finding fixes. The
focused regression, core-contract, and prepared-host suites run 2, 11, and 3
tests respectively and are green. The macOS ACL test was demonstrably red on
the old production and green on the descriptor-bound fix. A preliminary
read-only audit reported green, but it is explicitly not one of the required
formal tracks.

The three formal tracks inspected exact candidate `d59c7a5`. Documentation was
green. API/tests/portability found that valid fixtures inherited ambient modes
and could become group/other writable under `umask 000` or `002`; every valid
fixture now receives explicit `0700`, with isolated `umask 000` validation.
Security/resources/authority found that macOS mode `0700` does not exclude an
extended ACL granting or inheriting non-owner authority. The fix exact-pins the
target-macOS-only `calcifer-macos-acl` 0.1.0 descriptor API. The initial fix
rejected any ACL or ACL read failure on every retained state directory,
including a new suffix after permission normalization. A real
`everyone allow search` ACL regression keeps mode `0700`, was red before the
fix, and is green after it.
The exact-pinned crate has no normal dependencies, is locked to crates.io
checksum `d623f1bbaccbe0d1c6a9e4d2366feef6e179ac4e235aa86342601caf29358df4`,
and its published source at upstream commit `24a15cc4f7c46802d93d2f9cc93e45e1d5a5313e`
was inspected in full. The product crate continues to forbid unsafe Rust; the
dependency isolates its bounded native ACL parsing/FFI behind a safe
`BorrowedFd` API. Dependency policy and vulnerability gates are green.

The first finding-fix rereview then found that rejecting every entry also
rejected the protective `group:everyone deny delete` ACL on an ordinary macOS
`HOME`, breaking the documented fallback. The refined policy accepts only an
empty ACL or entries with exact `DENY` tag, zero flags, and exact `DELETE`
permission while requiring zero ACL-level flags. `ALLOW`, unknown tags/flags,
other or combined permissions, malformed data, and read errors still fail
closed. An independent HOME-fallback regression at `bb2a856` is red on the
reject-any implementation and green on the refinement.

All three tracks must now rereview this exact finding-fix candidate. Every
additional confirmed finding must be fixed, and all three must report green
together on the same exact behavior SHA before feature delivery.

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

Three exact-SHA formal rereviews, any resulting fixes and further rereviews,
full local gates, exact feature workflows, a fast-forward without force to `main`,
exact `main` workflows, and final delivery evidence remain pending. Session
lifecycle and reset/new-incarnation behavior, the remaining native tools, CLI
expansion, release-binary end-to-end host evidence, and compatibility promotion
remain open. No package or GitHub release is authorized.
