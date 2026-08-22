# Milestone 03 native root-selection review 01

Status: **REPLACEMENT SEAL GREEN — feature record and delivery pending**

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
  host evidence finding, fixed at `f1dc4751`
- Second exact finding-fix behavior candidate:
  `f1dc47517d5b2d6d37628be4eb2ab51871e20b5d`
- Second API/tests/portability rereview on `f1dc4751`: **GREEN**
- Second security/resources/authority rereview on `f1dc4751`: **GREEN**
- Second documentation/evidence rereview on `f1dc4751`: **GREEN**
- Adversarially green behavior:
  `f1dc47517d5b2d6d37628be4eb2ab51871e20b5d`
- Full local gates on `f1dc4751`: **GREEN**
- First documentation seal: `03fa9bae4a51ed3c0850ea6a777ddc093648449a`
- First-seal feature CI run
  [`32588948956`](https://github.com/distributedstatemachine/machine-god/actions/runs/32588948956):
  **NOT GREEN**; all four native Linux/macOS test jobs and dependency gates
  passed, but the Linux quality job found three target-specific strict-Clippy
  diagnostics
- First-seal benchmark-evidence run
  [`32588948975`](https://github.com/distributedstatemachine/machine-god/actions/runs/32588948975):
  **GREEN**
- Cross-platform lint normalization: `90d8f96e934d39b918ba7c627ecd08b5dd7152c5`
- Linux no-feature cross-target strict Clippy and full local macOS gates after
  `90d8f96`: **GREEN**
- Exact final rereview candidate:
  `72cf64f63e0dfa30bc1ee21d8aca16550e819c21`
- Final API/tests/portability rereview on `72cf64f6`: **GREEN**
- Final security/resources/authority rereview on `72cf64f6`: **GREEN**
- Final documentation/evidence rereview on `72cf64f6`: **GREEN**
- Adversarially green post-lint candidate:
  `72cf64f63e0dfa30bc1ee21d8aca16550e819c21`
- Replacement documentation seal:
  `f08dbd9eb2da81848b8eefb2d218006a64575835`
- Exact replacement-seal feature CI run
  [`32589778343`](https://github.com/distributedstatemachine/machine-god/actions/runs/32589778343):
  **GREEN**
- Exact replacement-seal feature benchmark-evidence run
  [`32589778374`](https://github.com/distributedstatemachine/machine-god/actions/runs/32589778374):
  **GREEN**
- Feature-evidence record and exact workflows: this documentation-only commit;
  exact SHA and runs reported at handoff
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
are fixed. All three formal tracks were green together on exact behavior SHA
`f1dc47517d5b2d6d37628be4eb2ab51871e20b5d`. The first documentation seal's
Linux feature CI then found three target-specific lint diagnostics while every
native Linux/macOS test job and benchmark evidence remained green. The portable
source normalization is present at `90d8f96`, and its full local macOS plus
Linux cross-target lint gates are green. Because production source changed,
all three tracks rereviewed exact candidate `72cf64f6`; all are green together.
Replacement documentation seal `f08dbd9e` is green under exact feature CI run
`32589778343` and benchmark-evidence run `32589778374`. The feature-evidence
record, `main`, and final-delivery gates remain pending.
Milestone 03 remains `IN PROGRESS`.

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

All three tracks rereviewed exact finding-fix SHA `f1dc4751` and reported green
together. API/portability confirmed the Rustdoc, ambient-umask, source-
compatibility, platform-cfg, and both ACL regression boundaries. Security
confirmed the exact deny-delete policy restores ordinary macOS `HOME` without
granting authority and that ALLOW, unknown, malformed, and read-error policy
remains descriptor-bound and fail-closed. Documentation confirmed the contract,
test counts, maintained summaries, evidence, deferred scope, links, and
Rust-product/Zig-benchmark boundary.

The first documentation seal at `03fa9ba` then exposed a Linux-only strict-
Clippy portability gap: two `u32::from(st_mode)` conversions are nontrivial on
macOS but useless on Linux, while the Linux no-op ACL validator unnecessarily
wrapped `Ok(())`. Native x86_64/aarch64 Linux and macOS test jobs, dependency
gates, and benchmark evidence were green on that seal. Fix `90d8f96` widens the
portable mode comparison to `u64` and compiles descriptor ACL validation only
on macOS; it changes no accepted path, mode, ACL, error, or public API behavior.
The exact failing Linux cross-target Clippy command, strict all-target/all-
feature local Clippy, all-feature workspace tests, and workspace documentation
are green. The required three-track final rereview inspected exact post-fix
candidate `72cf64f63e0dfa30bc1ee21d8aca16550e819c21`. API/tests/portability
confirmed the widening preserves unsigned permission masks on Linux and macOS,
the exact formerly failing Linux Clippy command is green, all 16 focused tests
pass, and public API, cfg resolution, error/debug contracts, and source
compatibility are unchanged. Security/resources/authority confirmed macOS ACL
validation remains active and fail-closed at both call sites while Linux only
loses its semantic no-op, with retained descriptors, bounded creation, root
disjointness, credential ordering, and redaction unchanged. Documentation and
evidence review confirmed the historical lineage, exact workflow outcomes,
test counts, maintained summaries, pending delivery status, and documentation-
only review exemption. All three tracks reported **GREEN** on the same exact
SHA and edited no files.

The replacement documentation-only seal and delivery records will update evidence and
status text only. Per the delivery workflow and the user's explicit instruction,
those documentation-only commits are not adversarially reviewed after
production behavior is already green.

## Candidate documentation checks

The post-`90d8f96` candidate's local gates pass:

- `cargo +1.94.1 fmt --all -- --check`;
- exact Rust/Cargo 1.94.1 strict workspace all-target/all-feature Clippy;
- default and all-feature workspace tests plus workspace documentation tests;
- 129 Python tests with eight expected macOS skips;
- both dependency-policy and vulnerability audits;
- native macOS focused ACL tests, available Apple/FreeBSD/WASI checks, and the
  release CLI smoke test; and
- `git diff --check`.

The Linux cross-target command that failed remotely also passes exactly:
`cargo +1.94.1 clippy --locked -p machine-god-native --lib
--no-default-features --target x86_64-unknown-linux-gnu -- -D warnings`.
These local checks are not replacement feature-workflow, benchmark, or `main`
evidence.

## Remaining scope

Exact feature-evidence-record workflows, a fast-forward without force to
`main`, exact `main` workflows, and final delivery evidence remain pending.
Session lifecycle and
reset/new-incarnation behavior, the remaining
native tools, CLI expansion, release-binary end-to-end host evidence, and
compatibility promotion remain open. No package or GitHub release is authorized.
