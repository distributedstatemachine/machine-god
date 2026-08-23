# Milestone 03 native `delete_file` review 01

Status: **IN PROGRESS — cycle 4 remediated; replacement review pending**

## Base and contract gate

- Exact delivered base:
  `719a9bded86fd7ce394d482798b9064c736f43ab`.
- Integration branch: `agent/m03-delete-file`.
- Normative contract: [`delete-file.md`](../delete-file.md).
- Contract commit: `78ed6292386f86e5807bcf72591d6cb5d9f45c45`.
- Exact contract CI: workflow `32652361712`, green across all six jobs.
- Exact contract benchmark evidence: workflow `32652361692`, green across both
  jobs with two nonexpired exact-SHA artifacts.

The exact base is green under feature CI `32651168514` across all six jobs and
feature benchmark workflow `32651168515` across both jobs with two nonexpired
exact-SHA artifacts. `main` was fast-forwarded without force from
`c1268fdf463e11242b7b916add70675ae91ed115` to the exact base and is green
under main CI `32651488265` across all six jobs and main benchmark workflow
`32651488282` across both jobs with two nonexpired exact-SHA artifacts.

This contract commit is documentation-only and is exempt from adversarial
review under the user's explicit instruction. Its workflows establish only the
frozen documentation boundary, not implementation, behavior, compatibility,
performance, equivalence, or delivery. Production and independent evidence are
now composed through exact local-gate precursor
`5e340155f9a38b81a2812942d6ad0a796164beb5`.

## Frozen boundary

The contract freezes:

- library-only deletion of exactly one existing confined regular file or empty
  directory, with no recursion, root deletion, content read, enumeration,
  creation, symlink following, external path, or CLI change;
- strict `{"path":string}` input, delivered mutation-path normalization, exact
  `FilesystemAccess::Delete`, and independent requested/canonical path,
  component, serialized-input, and serialized-result limits;
- exact path-only success, public symbols, construction errors, fixed redacted
  tool errors, retryability, and Linux/macOS platform scope;
- retained-root and parent descriptor traversal, no-follow metadata, complete
  parent/target identity and type revalidation, and a final cancellation check
  immediately before exactly one type-directed `unlinkat`;
- no `unlinkat` retry after `EINTR`, cancellation-ignoring bounded parent sync,
  and explicit commit-ambiguity recovery semantics;
- the portable final check-to-delete different-entry race, retained-parent
  movement, hard-link/open-descriptor survival, and pathname recreation; and
- exact eight-tool alphabetical reference-host composition with one additional
  retained descriptor clone.

There is no staging, ACL, chmod, content buffer, rename, recursive walk, or
directory enumeration. The slice adds no new dependency, benchmark workload,
compatibility promotion, product-performance claim, or fx-equivalence claim.

## Parallel ownership

- Production owns native implementation, exports, prepared-root/reference-host
  wiring, and any narrow existing-`Delete` contract regression.
- Independent tests own direct, engine, host, portability, fault, race, bounds,
  cancellation, and unsupported-target evidence.
- Documentation owns the normative contract, plan/index status, and this
  lineage record.

Owners use isolated worktrees and non-overlapping files. Component commits are
not behavior candidates independently; the integration branch must compose all
required behavior and evidence before formal review.

## Required production evidence seams

The real execution path must expose statically dispatched, deterministic seams
for:

- initial and final root/intermediate opens;
- ordinal-aware descriptor `fstat` and no-follow target `statat` calls;
- checkpoints after initial validation and after final validation;
- the actual `unlinkat` flags and syscall outcome;
- the checkpoint immediately after a real deletion; and
- parent `fsync`, including cumulative interruption outcomes.

Production behavior must remain identical with the no-op evidence type. No
global mutable seam, release-path dynamic dispatch, unbounded retry, or
test-only alternate executor is accepted.

## Required independent evidence

- [x] Exact public API, constants, schema, descriptions, result, construction
  taxonomy, fixed tool errors, retryability, `Display`, and redaction.
- [x] Exact/one-over requested and canonical path bytes, 256 components,
  65,536-byte serialized arguments, and 16,384-byte serialized result.
- [x] Effect-free strict preparation, denied execution before lookup, exact
  `Delete` capability, and canonical policy/direct-execution agreement.
- [x] Native regular-file and empty-directory deletion without content reads or
  enumeration; nonempty directory and root removal rejection.
- [x] Missing ancestors/targets, all symlink positions, FIFO/socket/device/
  other special objects, and outside sentinels fail closed without blocking.
- [x] Retained-root changes, complete parent/target rewalk and revalidation,
  moved retained parent, final different-entry race, hard links/open
  descriptors, and post-success pathname recreation.
- [x] Root/intermediate-open and ordinal `fstat`/`statat` faults, exact type
  flags, one actual `unlinkat`, post-delete checkpoint, and parent-sync faults
  all route through the real production path.
- [x] `unlinkat` success, definitive failures, and `EINTR` have exact mappings;
  interruption makes no second delete call; parent sync has one cumulative
  16-interruption ceiling and postcommit failures are nonretryable ambiguity.
- [x] Cancellation is covered through both traversal/validation passes, at the
  exact final pre-delete boundary, and after a real delete, plus inert-until-
  poll, drop, and core same-poll unknown-result recovery.
- [x] The exact eight-tool alphabetical catalog and original-plus-seven-clone
  retained identity are green without changing CLI bytes.
- [x] Native Linux/macOS, FreeBSD/WASI compilation, active unsupported-target,
  seven-tool regression, no-unsafe, docs, dependency, compatibility, and fresh
  release-smoke gates are green.

## Composed lineage and local gate

Production component `d31d0656528988f57caaecbacf15453f129ab27e`, independent
black-box component `2d2d8502a7b12ac6d9baeb983b06e604b58b2cde`, private
precommit component `e36cbd780d47fc847121ac3da75ec8a4649cd11e`, and private
commit-race components `f9464d736940b87a1e38948244d871d4497892eb`,
`829fff21c447dd7333b8042431932589e51f0801`, and
`af3d0ba7cd8ada7aa9b80794e786782dc5ba33df` compose through exact clean
precursor `5e340155f9a38b81a2812942d6ad0a796164beb5`. Deterministic evidence exposed
and closed the `ELOOP` and macOS `EPERM` type-race mappings before formal
review.

Under Rust and Cargo 1.94.1, formatting, workspace all-target/all-feature
warnings-denied Clippy, workspace tests, and two doctests are green. Focused
totals are 19 default-feature and 20 all-feature private tests, 19 direct, five
engine, and seven reference-host tests. Discovery reports 728 default-feature
tests, 778 all-feature tests, and zero benchmarks.

All 130 Python tests pass with eight expected macOS skips. Pinned-fx
`b1774fbf6c7602b503026f96f6e960e946c692ef` compatibility, cargo-deny 0.20.2,
cargo-audit 0.22.2, Linux/FreeBSD/WASI gates, and Node's active unsupported test
1/1 are green. Documentation integrity is 64 Markdown files, 445 inline links,
295 repository-relative links, and zero missing targets. Diff/no-unsafe/Cargo/
CLI checks are clean. A fresh locked arm64 Mach-O release CLI has SHA-256
`d5e91bac9cf07f389b98341ed0532d54d666f8aff2b92ffbd01f4a65cdfd8751`
and passes bare, help, and status smoke paths.

The tree-identical behavior marker
`7c6f7eed407f93d2ae335e6e3b5b4ad099a615cf` became formal cycle 1's exact
candidate. All three fresh tracks reviewed detached clean worktrees at that
same SHA and reported **NOT GREEN**. No candidate or delivery claim is made.

## Formal adversarial cycle 1

Exact candidate: `7c6f7eed407f93d2ae335e6e3b5b4ad099a615cf`.

1. Correctness/API: **NOT GREEN**, with two medium findings. Retained-root
   acquisition and linked-root metadata discarded `EACCES`/`EPERM`, violating
   the fixed nonretryable permission taxonomy in both validation phases. The
   macOS regular-file `EPERM` diagnostic metadata call also bypassed the frozen
   cancellation checks and could suppress precommit cancellation after a
   definitive failed unlink.
2. Filesystem/robustness: **NOT GREEN**, with two medium findings. It confirmed
   the retained-root permission mismatch independently and demonstrated that
   empty-flag `unlinkat` can remove a final-window symlink replacement, while
   the contract incorrectly characterized type changes as generally failing.
3. Performance/concurrency: **NOT GREEN**, with one medium finding. Serialized
   argument accounting preceded the requested path's 4,096-byte rejection, so
   a direct caller could force arbitrarily input-sized synchronous JSON string
   scanning before the fixed path bound fired.

The four unique remediations are: validate the requested path bound before
serialized-size accounting; preserve permission errors across retained-root
operations in either phase; make the macOS diagnostic metadata cancellation-
aware with cancellation precedence after a definitive noncommit; and freeze
and test the portable file-class replacement boundary for symlink, FIFO, and
socket entries with referent/sentinel preservation. A new candidate receives
the complete local gate and three entirely fresh same-SHA reviewers.

## Cycle 1 remediation local gate

Exact remediation: `60e81a633557bc90aca01e3579782340c7c154c9`.

The production path now validates the requested path bound before serialized
accounting, preserves `EACCES`/`EPERM` for retained and linked-root operations
in both phases, and routes the macOS post-`EPERM` target diagnosis through the
cancellation-aware ordinal metadata wrapper. Independent regressions cover
far-oversized preparation and direct execution, real post-construction root
mode loss with restoration, both permission errnos at every root operation,
cancellation during the second native macOS target `statat`, and final-window
symlink/FIFO/socket replacement with displaced and unrelated sentinels.

Rust/Cargo 1.94.1 formatting, workspace all-target/all-feature warnings-denied
Clippy, workspace tests, and two doctests are green. Focused totals are 22
default-feature and 23 all-feature private tests, 20 direct, five engine, and
seven reference-host tests. Discovery reports 732 default-feature tests, 782
all-feature tests, and zero benchmarks.

The 130-test Python harness passes with eight expected macOS skips. Pinned-fx
compatibility, cargo-deny 0.20.2, cargo-audit 0.22.2 over 1,225 advisories and
175 dependencies, Linux/FreeBSD/WASI gates, and Node's active unsupported test
1/1 are green. Documentation integrity remains 64/445/295/0; the base diff has
zero added unsafe Rust, Cargo metadata changes, or CLI source changes. The
optional all-feature Linux cross-build remains blocked in `aws-lc-sys` by the
host's missing Linux C sysroot before product Rust. A fresh locked 319,152-byte
arm64 Mach-O CLI has SHA-256
`d143cb7ef8ba0871a4449cd1f3a6ebb868dcb0f43f433819ea5110698e260304`
and passes bare, help, and unavailable-environment status smoke paths with
empty stderr.

This exact local gate does not make cycle 1 green. A tree-identical marker must
receive three fresh formal cycle-2 reviews before any behavior-green claim.

## Formal adversarial cycle 2

Exact candidate: `88026f10ed8c194c7160a754f226241c276579fc`, tree-identical
to the recorded cycle-1 remediation local gate.

1. Correctness/API: **NOT GREEN**, with three medium findings and one low
   finding. It found that failed open/metadata calls bypassed their
   after-operation cancellation check, the macOS `EPERM` diagnosis compared
   only directory type rather than complete validated identity, and non-root
   revalidation `EACCES`/`EPERM` lost the fixed permission taxonomy. Public
   Rustdoc also retained the obsolete same-type-only final-window disclosure.
2. Filesystem/robustness: **NOT GREEN**, with the same three medium findings.
   Its deterministic scenarios include cancellation plus a failed evidence
   operation, a file-to-directory `EPERM` followed by diagnostic-window removal
   or another replacement, and revalidation mode loss at non-root sites.
3. Performance/concurrency: **GREEN** with zero findings. It confirmed the
   requested path's constant-time byte-limit rejection precedes serialized JSON
   traversal and found no remaining bounded-work, cancellation-frequency,
   resource-lifetime, or concurrency defect.

The four unique remediations require every root/parent open and metadata
operation to save its result, always run the after-check, and give cancellation
precedence; complete identity-aware macOS diagnosis with explicitly frozen
diagnostic errno precedence; permission-errno precedence at every root, parent,
and target site in both phases; full combined error/cancellation, permission,
and diagnostic race matrices; and corrected public Rustdoc. Cycle 2 remains
historically **NOT GREEN**. A new exact candidate receives the complete local
gate and three entirely fresh reviewers.

## Cycle 2 remediation local gate

Exact remediation: `225e9617a8a8f469d663693b61cc4f9b97af8094`.

Production now captures each root/parent open and metadata result, always runs
the matching after-check, and gives cancellation precedence before mapping a
saved noncommit error. Every `EACCES`/`EPERM` at a root, parent, or target site
in either phase has permission precedence. The macOS `EPERM` diagnosis receives
and compares complete target identity and has the contract's fixed cancellation,
absence/type-change, permission, and other-error precedence. Public Rustdoc
discloses the complete flags-compatible replacement boundary.

Independent evidence injects cancellation plus `EIO` at every traced open,
`fstat`, and `statat` site/ordinal and requires the after-check, cancelled result,
zero unlink/sync, and an intact target. It covers both permission errnos at
every site in both phases, real intermediate mode loss with RAII restoration,
macOS directory-to-absent/symlink/FIFO/socket/different-file diagnostic races,
unchanged identity, the complete diagnostic errno matrix, and cancellation
plus diagnostic failure.

Rust/Cargo 1.94.1 formatting, workspace all-target/all-feature warnings-denied
Clippy, workspace tests, and two doctests are green. Focused totals are 28/29
private, 20 direct, five engine, seven host, and one core-contract test;
discovery is 738/788 with zero benchmarks. Python 130, pinned-fx, cargo-deny
0.20.2, cargo-audit 0.22.2 over 1,225 advisories and 175 dependencies,
Linux/FreeBSD/WASI plus active Node 1/1, and documentation 64/445/295/0 pass.
The 16-file delivered-base diff is +6,172/-63 with zero added unsafe Rust and
no Cargo or CLI changes. Optional all-feature Linux cross remains a host C-
sysroot failure in `aws-lc-sys` before product Rust. A fresh locked 319,152-byte
arm64 Mach-O CLI has SHA-256
`951ff7ce945a6fa446dfd87a7d54a6dd962776a8a021d4af6e68d6bd18e963e8`
and passes bare/help/unavailable-status smoke with empty stderr.

Cycle 2 remains historically not green. A tree-identical marker must receive
three fresh cycle-3 reviews before any behavior-green claim.

## Formal adversarial cycle 3

Exact candidate: `24f851d2d3db21735124729bb1b0a14adf7ae864`, tree-identical
to the recorded cycle-2 remediation local gate.

1. Correctness/API: **NOT GREEN** with one low finding. The normative taxonomy
   groups read-only-filesystem failures with permission failures at every
   validation site, but only `EACCES`/`EPERM` received permission precedence;
   injected `EROFS` still mapped by phase. No other finding remained.
2. Filesystem/robustness: **NOT GREEN** with one low evidence gap. The required
   hostile-umask deletion case was not retained. Manual execution under a
   restrictive umask passed; an extreme `0777` run failed only during fixture
   construction before product execution. It found no production protocol,
   confinement, mapping, cancellation, syscall, durability, descriptor, or
   portability defect.
3. Performance/concurrency: **GREEN** with zero findings. It reconfirmed bounded
   path/serialization/traversal/fd/unlink/sync/diagnostic behavior, cancellation
   ordering, static evidence, fixed host scale, and accurate nonclaims.

Remediation classifies `EROFS` with permission at every validation mapping and
adds it to the full phase/site/ordinal matrix. Independent public evidence uses
an isolated child process with a hostile umask, explicitly repairs only the
fixture workspace mode, creates mode-zero file and empty-directory targets,
and proves exact deletion while unrelated sentinels remain unchanged. Cycle 3
remains historically **NOT GREEN**; a new exact candidate receives complete
local gates and three fresh reviewers.

## Cycle 3 remediation local gate

Exact remediation: `77884a9fceed6268cbdbec1310de3f94a9c5a230`.

The execution-time permission helper now includes `EROFS` with `EACCES` and
`EPERM` at every root, parent, target, and unlink mapping without changing
construction taxonomy. The complete phase/site/ordinal matrix exercises all
three errnos. An isolated public child-process regression sets `umask 0777`,
repairs only its workspace directory to mode `0700`, creates mode-zero regular
file and empty-directory targets, proves exact deletion results, and preserves
file and directory sentinels outside the workspace.

Rust/Cargo 1.94.1 formatting, workspace all-target/all-feature warnings-denied
Clippy, workspace tests, and two doctests are green. Focused totals are 28/29
private, 21 direct, five engine, seven host, and one core contract; discovery
is 739/789 with zero benchmarks. Python 130, pinned-fx compatibility, cargo-
deny 0.20.2, cargo-audit 0.22.2 over 1,225 advisories and 175 dependencies,
Linux/FreeBSD/WASI plus active Node 1/1, and docs 64/445/295/0 pass. The 16-file
delivered-base diff is +6,404/-63 with zero added unsafe Rust and no Cargo or
CLI changes. Optional all-feature Linux cross remains a host C-sysroot failure
in `aws-lc-sys` before product Rust. A fresh locked 319,152-byte arm64 Mach-O
CLI has SHA-256
`951ff7ce945a6fa446dfd87a7d54a6dd962776a8a021d4af6e68d6bd18e963e8`
and passes bare/help/human-status/JSON-status smoke with empty stderr.

Cycle 3 remains historically not green. A tree-identical marker must receive
three fresh cycle-4 reviews before any behavior-green claim.

## Formal adversarial cycle 4

Exact candidate: `0b732d2746d5c821a5294901f8b4cc641bc98530`, tree-identical
to the recorded cycle-3 remediation local gate.

All three tracks reported **NOT GREEN** with the same single medium finding and
no others. A definitive non-`EINTR` `unlinkat` failure flowed directly to errno
or macOS diagnostic mapping without checking cancellation raised during that
syscall. This made direct-tool cancellation precedence depend on platform and
errno even though only success and `EINTR` are postcommit/ambiguous outcomes
that ignore later cancellation.

Remediation checks cancellation after the syscall/evidence hook and before all
definitive failure mappings, with no retry or sync. Independent production-seam
evidence covers representative file `EIO`, permission `EACCES`/`EPERM`/`EROFS`,
target-change `ENOENT`/type mismatch, and directory `ENOTEMPTY`/`EEXIST`
outcomes; every cancelled case retains the target and sentinels, makes exactly
one delete call with exact flags, and performs zero syncs. Existing success and
`EINTR` tests continue to prove cancellation is ignored after the commit/
ambiguity boundary. Cycle 4 remains historically **NOT GREEN**; a new exact
candidate receives complete gates and three fresh reviewers.

## Cycle 4 remediation local gate

Exact remediation: `4273de513007175be94829aef85aaaa0d09bc02c`.

Production now checks cancellation after a definitive non-`EINTR` deletion
failure and its evidence hook, before every errno or macOS diagnostic mapping.
Success and `EINTR` remain beyond the cancellation-replacement boundary. A
ten-case production-seam matrix covers file `EIO`, `EACCES`, `EPERM`, `EROFS`,
`ENOENT`, `ENOTDIR`, `EISDIR`, and `ELOOP`, plus directory `ENOTEMPTY` and
`EEXIST`; every cancelled case returns the fixed cancellation result, retains
target and sentinel state, makes exactly one delete call with exact flags, and
performs zero syncs.

Rust/Cargo 1.94.1 formatting, workspace all-target/all-feature warnings-denied
Clippy, workspace tests, and two doctests are green. Focused totals are 29/30
private, 21 direct plus the hostile-umask child, five engine, seven host, and
one core `Delete` contract test. Discovery is 740 default / 790 all-feature
tests including two doctests, with zero benchmarks.

Python 130 with eight expected skips, pinned-fx compatibility, cargo-deny
0.20.2, cargo-audit 0.22.2 over 1,225 advisories and 175 dependencies,
Linux/FreeBSD/WASI plus active Node 1/1, and documentation 64/445/295/0 pass.
The clean 16-file delivered-base diff is +6,582/-63 with zero added unsafe Rust
and no Cargo or CLI changes. Optional all-feature Linux cross remains a host
C-sysroot failure in `aws-lc-sys` before product Rust. A fresh locked
319,152-byte arm64 Mach-O CLI has SHA-256
`126ecc47857cb327e3b483daecf9c50ce6b04585f4cdaed60e6f20cb9f82b107`
and passes bare/help/human-status/JSON-status smoke with exact stdout and empty
stderr.

Cycle 4 remains historically not green. This exact local gate does not
establish behavior approval or delivery; a tree-identical marker must receive
three fresh cycle-5 reviews before any behavior-green claim.

## Formal adversarial protocol

After all implementation and evidence compose into one exact behavior SHA,
three fresh agents review that same SHA for:

1. correctness and public API;
2. filesystem behavior and robustness; and
3. performance and concurrency.

Each track must explicitly report **GREEN** with zero findings or **NOT GREEN**
with every finding. Any finding is remediated, all local gates rerun, and all
three tracks restart with fresh agents on the same new SHA. A later docs-only
seal or delivery record is exempt from another adversarial cycle under the
user's instruction, while exact feature and `main` workflows remain mandatory.

## Pinned input and non-claims

Pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef` was observed
to accept a regular file and empty directory but has broader pathname-based
behavior. Machine-god's stricter bounds, retained-descriptor confinement,
revalidation, cancellation, redacted errors, durability, and race disclosure
are deliberate differences.

The contract makes no compatibility promotion, benchmark-workload change,
product-performance claim, or fx-equivalence claim. Zig remains solely the
pinned upstream benchmark build input; the implementation remains Rust.
