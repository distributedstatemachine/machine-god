# Milestone 03 native `edit_file` review 01

Status: **IN PROGRESS — composed local gates green; formal review pending**

## Base and contract gate

- Exact delivered base:
  `242adfed4be717baf7cd07275aae40ec8a3637f6`.
- Integration branch: `agent/m03-edit-file`.
- Normative contract: [`edit-file.md`](../edit-file.md).
- Contract commit: `bb0c381f8b044f7849bef80cc482034e1dd57ecf`.
- Exact contract CI: workflow `32634883133`, green.
- Exact contract benchmark evidence: workflow `32634883139`, green.

The contract commit changed documentation only and was exempt from adversarial
review under the user's explicit instruction. Its green remote workflows
established only the documentation boundary; they did not establish production
behavior, compatibility, performance, equivalence, or delivery.

## Frozen boundary

The contract freezes:

- strict effect-free `{path,old_string,new_string}` preparation;
- `FilesystemAccess::Edit`, serialized as `edit`, rather than widening the
  delivered no-read `Write` authority;
- independent 4,096-byte/256-component path, 49,152-byte old/new/preimage/
  postimage, 65,536-byte serialized-input, 8,192-byte chunk, eight-name,
  16-interruption, 31-entropy-call-per-name, and 393,216 matcher-work limits;
- valid-UTF-8 existing content, exact byte matching, overlapping ambiguity,
  NUL acceptance, empty replacement, and exact success path/byte count;
- Linux/macOS retained-descriptor, no-follow, existing-regular-file-only,
  same-parent staged atomic replacement with original ordinary rwx bits;
- bounded target/content/parent/staged-name revalidation, cancellation,
  cleanup, race disclosure, and post-rename ambiguity semantics; and
- exact seven-tool alphabetical reference-host composition.

Preparation remains effect-free, so this slice deliberately defers pinned fx's
preapproval preimage read and computed diff. It also defers external paths,
parent or file creation, binary/alternate-encoding edit, regex/patch/range/
multi-edit, metadata preservation beyond ordinary rwx bits, CLI changes,
non-Linux/macOS hardening, compatibility promotion, benchmark workload changes,
and any product-performance or fx-equivalence claim.

## Composed lineage

The production component was developed as the following additive lineage:

- `9c7976af836e8ab75fd945fd98f1671863e1bde3` — bounded native edit tool;
- `f90068ef90f386586ae19e96c68f89ba289ef4ef` — supported-target cfg gates;
- `af96e49026b28802cb7fa5169c5051e35d193a5b` — staging-setup cleanup;
- `4b0649a081dd61ae9a95bb39e142850961fa66b6` — complete stable-size reads;
- `83d3fcc5ceac4541d67900049c3376c2fed1b979` — final type and cancellation
  revalidation; and
- `1c41bccc204f0dab33673fc2bc8eea9e0059b62e` — formatting only.

Independent-test components were
`c528b9638ae2a645c9516c0ca284e5e704998219`,
`439eaac47bc803103b6d3fc85a2c830e3c2fa4e2`, and
`3c62933319f8bd638da589744580ca3da54c313d`.

Those components were composed on the integration branch as, in order:

- `af9319747f15cdb5c943f9b6d04a0fd65220e2f8` — production;
- `ffc964d33b9710b033c553f00a290edd46a58f5d` — independent tests;
- `9d227fa872480e15f23ce0b6b1f83e6cfc6465a9` — test formatting;
- `f5f218605379240c3f03650bece760cb285c2bfb` — cfg gates;
- `b127d232ff35cc298dd216486eaaac936fe8f81d` — staging cleanup;
- `23b1569c7e73b61441c65b30f997c115db715d44` — fault evidence;
- `6bc2e4e2bc17e7309b57c6ce49a028df951ab07f` — complete reads;
- `4c181f89bb6eeb627a447de7d61f060a511fe6a0` — type/cancellation
  revalidation;
- `dbee5e694363ae236116228881e6dab966a3fe53` — formatting only; and
- `ad16260447a0d0c6346b6d9d859783c9d4347c20` — complete-read evidence.

Benchmark-harness correction component
`e4eec3cac30ec923c19fa53a81e5b6ba9b81cfae` was integrated as exact clean
local-gate precursor `31ec79e000589c4fb34599be4aad4f90ea33974f`.
This documentation record follows that precursor. Because a commit cannot
self-record its own identifier, its documentation-only component SHA is
recorded after direct observation in the integration handoff.

## Required independent evidence

The check marks below record only evidence directly exercised on the composed
precursor. Unchecked items identify work for formal review or a subsequent
evidence commit; they do not imply a known product defect.

- [x] Exact public symbols, constants, schema/property descriptions,
  construction/tool errors, serialized forms, and redacted debug/display.
- [x] Strict requested/canonical input, path and component boundaries,
  independent text and serialized limits, empty old string, identical strings,
  and preparation with zero filesystem effects.
- [x] Exact `FilesystemAccess::Edit` capability and canonical policy/execution
  agreement, including denial before target read and strict direct execution.
- [x] Beginning/middle/end, Unicode, NUL, empty replacement, complete deletion,
  exact size boundaries, zero/one/two matches, and overlapping ambiguity.
- [x] Linear matcher accounting at exact/one-over injected budgets, public-cap
  headroom, and bounded cancellation polling with fixed failure mapping.
- [x] Existing regular-file-only behavior, missing target/parents, invalid UTF-
  8, oversize/growing content, ancestor/final symlinks, and special types.
- [ ] Exact original ordinary-rwx preservation under hostile umask, inode
  replacement, old-descriptor/new-path visibility, hard-link behavior, and
  deliberate nonpreservation of other metadata. The composed direct suite
  proves every listed behavior except hostile-umask independence.
- [ ] Retained-root changes and deterministic target identity/mode/content,
  staged-name, and parent races through the real production pipeline. Retained-
  root replacement/removal, target change, and staged-name swap are directly
  proven; a deterministic parent-race seam remains for review.
- [ ] Every read/match/construction/write/chmod/file-sync/rename/parent-sync
  fault with unchanged-target precommit and nonretryable postcommit ambiguity.
  Broad phase fault coverage is green, but the universal claim remains for
  formal audit.
- [ ] Cumulative interruption bounds, entropy partial progress and exhaustion,
  eight collisions, collision preservation, cleanup swaps, held-descriptor mode
  reset, and disclosed residue dual-failure behavior. All except the disclosed
  dual-failure residue outcome are directly exercised.
- [ ] Cancellation during both reads, matching, construction, traversal,
  entropy/staging, final verification, immediately before rename, unpolled/
  drop, and engine same-poll durable recovery. Multiple bounded phase cases are
  green; the complete phase matrix remains for formal audit.
- [x] Exact seven-tool alphabetical host catalog, original-plus-six-clone
  descriptor identity, complete `write_file` regression, native macOS behavior,
  Linux/FreeBSD/WASI compilation, and active unsupported behavior.
- [ ] Exact native Linux execution remains part of the formal and remote gate;
  the local Linux result is compilation evidence only.
- [x] Private production-helper evidence proves the exact 16,384-byte
  serialized-result guard because every public success payload is smaller.

## Composed local-gate evidence

Exact precursor `31ec79e000589c4fb34599be4aad4f90ea33974f` is locally green under
Rust and Cargo 1.94.1:

- focused suites pass 25 private production-helper tests, 23 direct tests, five
  engine tests, and seven reference-host tests;
- formatting, all-target/all-feature warnings-denied Clippy, workspace tests,
  and two doctests pass;
- discovery inventories 665 default-feature tests, 714 all-feature tests, and
  zero benchmarks;
- pinned-fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef`
  compatibility is green;
- cargo-deny 0.20.2 and cargo-audit 0.22.2 are green;
- Linux, FreeBSD, and WASI gates pass, while Node actively executes the WASI
  unsupported-target `edit_file` test 1/1;
- a freshly built locked arm64 Mach-O release CLI has recorded SHA-256 prefix
  `87f452`; bare, help, and status smoke paths pass;
- the pre-documentation tree contains 62 Markdown files, 437 inline links, 287
  repository-relative links, and zero missing targets; and
- no unsafe Rust was added, the whole-feature diff check passes, and the
  precursor worktree is clean.

The repository harness first failed honestly at 129 tests because its benchmark
compatibility helper sorted paths while the real Git command supplies source
order. Component `e4eec3cac30ec923c19fa53a81e5b6ba9b81cfae` preserves Git
order and adds the thirtieth benchmark-harness test. The final harness is green
at 130 tests: 122 pass and eight expected macOS skips. This is harness
correctness evidence, not a product-performance result.

## Formal review and delivery gates

This record is pre-adversarial. Three fresh agents must independently inspect
one subsequent exact composed behavior SHA for:

1. correctness and public API;
2. filesystem confinement, atomicity, durability, and robustness; and
3. performance, bounded work, cancellation, and concurrency.

Every confirmed finding is fixed and restarts all three tracks with fresh
agents on one new exact behavior SHA. Delivery then requires exact feature-
branch CI and benchmark workflows, a no-force fast-forward of `main`, and exact
`main` CI and benchmark workflows. Documentation-only records are exempt from a
separate adversarial cycle under the user's instruction, but their applicable
remote workflows remain required.

## Pending lineage

- Exact base: `242adfed4be717baf7cd07275aae40ec8a3637f6`
- Contract: `bb0c381f8b044f7849bef80cc482034e1dd57ecf`
- Contract CI: `32634883133`, green
- Contract benchmark evidence: `32634883139`, green
- Local-gate precursor: `31ec79e000589c4fb34599be4aad4f90ea33974f`
- Documentation component after composition: pending integration handoff
- Exact formal behavior candidate: pending
- Correctness/API track: pending
- Filesystem/robustness track: pending
- Performance/concurrency track: pending
- Behavior-green SHA: pending
- Documentation seal: pending
- Exact feature CI and benchmark evidence: pending
- No-force fast-forward `main`: pending
- Exact `main` CI and benchmark evidence: pending

The local precursor establishes neither formal candidate status nor
adversarial, feature-delivery, `main`, compatibility-promotion, equivalence, or
product-performance approval. Zig remains solely the pinned upstream fx
benchmark build input; the machine-god product implementation is Rust.
