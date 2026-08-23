# Milestone 03 native `edit_file` review 01

Status: **BEHAVIOR GREEN — delivery pending**

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

The remediated publication protocol keeps the stage at `0600` through the long
target rewalk and reread, but no longer treats mode alone as sufficient on
macOS. Immediately after exclusive creation it clears and verifies the held
stage has no ACL flags or entries before any write. Complete mutation-sensitive
staged-path, held-descriptor exact-content, and empty-ACL checks continue
through both deterministic race hooks and after final mode/sync. After rename
it ignores tool cancellation, stably rereads exact bytes through the retained
descriptor, rechecks the empty ACL and published path, and always attempts
parent sync; any verification or sync failure is nonretryable commit ambiguity.
This narrows but does not remove same-UID, root, final check-window, or post-
verification writer races.

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
Documentation component `b1210f395a25bc59590c3b4b0164fac56e96bca0` and
formal-preparation commit `3934d9d26ced78d5164e9ff2620c44ebb6480dd1`
produced exact cycle-1 candidate
`8fdb67892f34a0fbfbb90a54e8eda982159813bf`.

Cycle-1 remediation has the following exact lineage:

- production component `578ef3cf2061568d02a160fbe7a498203880b9e9`,
  integrated as `013016f276e023838ffe7ddf8a79121a3ee463a1`; and
- independent-test component `59471147817ed7520513fdf51041ec24c822bfe3`,
  integrated as exact composed precursor
  `482d33c0bc586ff594d5b0decc58de347cb9243e`.

Cycle-1 remediation documentation
`b02b4e9c1262042c7f0aa7fc5112520f8c406924` and review preparation
`5a985798b679e6cfaeeca61af1ced8da42c02bc1` produced exact cycle-2
candidate `f84bac87f472fc851eca670764657e5a31ce0256`.

Cycle-2 remediation has the following exact lineage:

- production component `22197389a521095132c02125726dbe67fbf06d1b`,
  integrated as `7900d97269341a9b8a46bcdcdb987279bc168e4d`; and
- independent-test component
  `65d40d99f4e026834a05778029800fa703c9379e`, integrated as exact composed
  behavior SHA `ab6841388838384e27e6299151d50bb83d2ec46e`.

Cycle-2 remediation documentation `1171daa` and formal preparation
`1c4be4b` produced the cycle-3 review candidate. Portable Linux ACL-adapter
lint correction `9ee7f3f` was refrozen as exact cycle-3 candidate
`da1537b229393007101264cd7bc8fd12ee393a3d`.

Cycle-4 remediation has the following exact lineage:

- production component `985b232731883f7a5c18f8f7cbba56dbedfc7c6e`,
  integrated as `1b9ffca9031c61625420279569670c1c80d2d750`; and
- independent-test component
  `3cbb8956477af30cf5f8d63f118e597793267efc`, integrated as exact composed
  remediation SHA `d0d188b39290a50f7f10d7e4665cf694abdfc460`.

Documentation-only component
`590ad386ca7c783d9fd463ab48531fa81025f11c` was integrated as exact parent
`12470d9e6c4c9301a0eeaef34e01a1ab31c84d07`. Formal freeze marker
`78d6fd7e0c42ec97f4f176e8378ab774c25893ca` is tree-identical to that parent
and is the exact cycle-4 behavior candidate reviewed below.

## Cycle-1 review result and remediation

All three fresh tracks reviewed exact candidate
`8fdb67892f34a0fbfbb90a54e8eda982159813bf`:

- correctness/API: **GREEN**, zero findings;
- filesystem/robustness: **NOT GREEN**, with one high finding that a same-
  inode, same-size staged mutation after the only content check could be
  published while returning success, plus one low finding that mandatory
  hostile-umask, parent-race, universal-fault, and cancellation evidence was
  incomplete; and
- performance/concurrency: **NOT GREEN**, with the same high staged-content
  race.

The production remediation closes the false-success path by preserving private
stage mode through long validation, rereading exact staged content after both
race hooks, syncing only after applying final mode near publication, and adding
the cancellation-ignoring exact published-content verification described
above. The independent component's three corruption regressions are red against
the failed candidate and green with the production remediation: two precommit
same-inode/same-size staged mutations fail without publication, while a real
rename followed by same-length held-inode corruption returns commit ambiguity
and still attempts parent sync.

Exact composed precursor `482d33c0bc586ff594d5b0decc58de347cb9243e`
passes 30 private production-helper tests, 24 direct tests, five engine tests,
and seven reference-host tests. Newly direct evidence also covers exact mode
preservation under hostile umask, cancellation at final verification, and the
disclosed publication into a retained parent moved outside its public path.
That precursor closed the cycle-1 defect and became the basis of the cycle-2
candidate recorded below.

## Cycle-2 review result and remediation

All three fresh tracks reviewed exact candidate
`f84bac87f472fc851eca670764657e5a31ce0256`:

- correctness/API: **GREEN**, zero findings;
- performance/concurrency: **GREEN**, zero findings; and
- filesystem/robustness: **NOT GREEN**, with one high and one low finding.

The high finding was macOS-specific: a same-parent stage created with
`open(O_EXCL, 0600)` could retain a file-inherited allow ACL, and the existing
`fchmod` operations did not clear it. The unsafe ACL could therefore survive
long staging and follow the inode through rename into publication. The low
finding was evidence incompleteness across deterministic root/intermediate
traversal, staged-creation error/cancellation, logical read phases, the final
mode/sync boundary, immediately after the real rename, and cleanup mode/unlink
dual failure. Cycle 2 explicitly confirmed that cycle 1's same-inode/same-size
staged-content corruption defect was fixed; neither green track reported a new
correctness, API, performance, bounded-work, cancellation, or concurrency
finding.

Production component `22197389a521095132c02125726dbe67fbf06d1b` closes the
high finding with the existing exact `calcifer-macos-acl` dependency. Through
the held staged descriptor it clears and verifies an empty ACL immediately
after exclusive creation and before any content write. Every later staged
content verification rechecks the ACL before and after its stable reread,
including after final mode and sync; published verification does the same after
rename. A clear/read/nonempty ACL failure is a retryable `edit_file_write_failed`
before rename and nonretryable `edit_file_commit_ambiguous` after rename.
Linux retains its supported behavior without compiling the macOS-only crate.

The component also adds a generic, statically dispatched `EditFileEvidence`
trait while preserving the existing execution wrapper. Its production no-op
implementation routes phase-labelled root/intermediate, target, and stage
opens; `pread`, `fstat`, and `statat`; checkpoints after RAII takes ownership of
the stage, after final staged sync, and immediately after successful real rename
and publication marking; plus an independently injectable cleanup mode/unlink
helper. The real pipeline and test evidence therefore share one control flow
without global state or a release behavioral fork.

Independent component `65d40d99f4e026834a05778029800fa703c9379e` adds nine
private helper tests, raising that suite from 30 to 39. The 24 direct, five
engine, and seven reference-host tests remain green. A real macOS regression
installs a file-inherited `everyone` allow ACL on the parent, proves an ordinary
child inherits it, then proves both the private staged inode and published inode
have empty ACLs while the parent retains its ACL and the original ordinary mode
and edited bytes are correct.

The phase matrix directly covers:

- initial and revalidation root/intermediate open error and cancellation;
- initial/revalidation target open and path-stat errors, plus read error,
  cumulative 16-interruption exhaustion, early EOF, and cancellation;
- stage-open error and error/cancellation immediately after staged RAII
  ownership, with unchanged target and no residue;
- staged/published descriptor-stat, path-stat, and read error, interruption
  exhaustion, and early EOF with exact retryable precommit versus nonretryable
  postcommit mapping and parent sync after publication;
- same-size corruption after final mode/sync and before rename;
- cancellation immediately after real rename, which is ignored while the
  published reread and exactly one parent sync complete successfully; and
- an independently failing cleanup mode reset followed by an attempted,
  independently failing unlink, proving the disclosed final-mode owned residue
  outcome without changing the original target.

## Cycle-3 review result

All three fresh tracks reviewed exact candidate
`da1537b229393007101264cd7bc8fd12ee393a3d`:

- correctness/API: **GREEN**, zero findings;
- performance/concurrency: **GREEN**, zero findings; and
- filesystem/robustness: **NOT GREEN**, with four low deterministic-evidence
  findings and no production atomicity defect.

The four low gaps were precise. The dual-failure cleanup regression injected a
helper rather than the actual RAII `StagedFile::drop` operations. The named
final-verification cancellation hook ran before the last complete staged
verification, not at the true verification-to-rename boundary. ACL evidence
did not independently inject clear failure, read failure, and unsafe nonempty
outcomes across creation, final staged verification, and publication. Finally,
phase-only fault selection could not choose later descriptor `fstat` calls by
call ordinal. Neither green track reported a correctness, API, performance,
bounded-work, cancellation, or concurrency finding.

## Cycle-4 remediation

Production component `985b232731883f7a5c18f8f7cbba56dbedfc7c6e` makes the
actual RAII guard generic over a statically dispatched
`EditFileCleanupEvidence`. Its `Drop` routes mode reset, descriptor `fstat`,
no-follow pathname `statat`, and unlink through the injected evidence while
retaining identity and regular-type checks. A failed mode reset does not skip
the unlink attempt, and cleanup cannot replace the primary pipeline error. The
production wrapper uses the native descriptor operations without global or
dynamic release state.

The same component adds `after_final_stage_verification` immediately after the
last staged content/ACL verification and before the final cancellation check
and rename. `clear_staged_acl` and phase-labelled `staged_acl_is_empty`
evidence defaults preserve the exact macOS descriptor ACL operations and the
Linux adapters while exposing creation, late-precommit, and published outcomes
to deterministic tests. Existing phase-labelled `fstat` calls remain separate
and observable.

Independent component `3cbb8956477af30cf5f8d63f118e597793267efc` adds
ordinal-aware fault ranges and per-phase/operation call counts, plus four
regressions:

- `pipeline_descriptor_and_late_fstat_faults_target_exact_ordinals` targets
  initial ordinal 2, revalidation ordinal 2, staged ordinals 18 and 19, and
  published ordinals 3 and 4 with exact pre/postcommit mappings;
- `pipeline_acl_clear_read_and_unsafe_outcomes_map_at_exact_ordinals` covers
  clear failure, creation read failure/nonempty, final staged ordinal-9 read
  failure/nonempty, and published ordinal-2 read failure/nonempty;
- `pipeline_cancellation_after_final_verification_precedes_rename_and_cleans_stage`
  proves cancellation at the actual boundary leaves the target unchanged,
  cleans the stage, and never publishes; and
- `pipeline_drop_attempts_dual_failure_cleanup_after_final_verification`
  drives the actual RAII `Drop`, proves mode reset failure is followed by
  descriptor/path identity observations and an unlink attempt, preserves the
  primary precommit error, and directly observes the disclosed residue.

Exact integrated remediation SHA
`d0d188b39290a50f7f10d7e4665cf694abdfc460` passes 43 private `edit_file`
tests, 24 direct tests, five engine tests, and seven reference-host tests. The
delivered `write_file` regressions remain green at 30 private, 25 direct, and
five engine tests. This documentation-only remediation record received no
separate adversarial review under the user's explicit instruction.

## Cycle-4 review result

All three fresh tracks reviewed exact tree-identical candidate
`78d6fd7e0c42ec97f4f176e8378ab774c25893ca`:

- correctness/API: **GREEN**, zero findings;
- filesystem/robustness: **GREEN**, zero findings; and
- performance/concurrency: **GREEN**, zero findings.

The filesystem/robustness track explicitly confirmed closure of all four
cycle-3 evidence gaps. The actual generic RAII `StagedFile::drop` path attempts
both mode reset and unlink under dual failure while retaining descriptor/path
identity and regular-type gates. `after_final_stage_verification` is reached
after the last stable content and ACL verification and immediately before the
last cancellation check and rename. ACL clear, read failure, and unsafe
nonempty outcomes are independently selected at creation, late-precommit, and
publication ordinals with exact pre/postcommit mapping and postpublication
parent sync. Ordinal-aware descriptor `fstat` injection targets the intended
initial, revalidation, late staged, and published calls while retaining the
bounded interruption semantics.

The green tracks reported no correctness, API, filesystem, robustness,
bounded-work, cancellation, concurrency, or review-blocking finding. This is a
behavior-green result, not feature delivery, compatibility-status promotion,
product-performance evidence, or an fx-equivalence claim.

## Required independent evidence

The check marks below record evidence exercised through exact reviewed cycle-4
candidate `78d6fd7e0c42ec97f4f176e8378ab774c25893ca`. Unchecked items identify
remote delivery work and do not imply a known product defect.

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
- [x] Exact original ordinary-rwx preservation under hostile umask, inode
  replacement, old-descriptor/new-path visibility, hard-link behavior, and
  deliberate nonpreservation of other metadata.
- [x] Retained-root changes and deterministic target identity/mode/content,
  staged-name, and moved-parent races through the real production pipeline,
  including the disclosed retained-parent publication behavior.
- [x] Phase-exact target/staged/published open, descriptor-stat, path-stat, and
  read faults plus existing match, construction, write, chmod, file-sync,
  rename, and parent-sync failure evidence, with unchanged-target precommit and
  nonretryable postcommit ambiguity.
- [x] Cumulative interruption bounds, entropy partial progress and exhaustion,
  eight collisions, collision preservation, cleanup swaps, held-descriptor mode
  reset, and the directly exercised disclosed residue dual-failure behavior.
- [x] Cancellation during both reads, matching, construction, initial and
  revalidation root/intermediate traversal, entropy/staging, final
  verification, immediately before and after real rename, unpolled/drop, and
  engine same-poll durable recovery. Post-rename cancellation is ignored while
  published verification and parent sync finish.
- [x] Exact seven-tool alphabetical host catalog, original-plus-six-clone
  descriptor identity, complete `write_file` regression, native macOS behavior,
  Linux/FreeBSD/WASI compilation, and active unsupported behavior.
- [ ] Exact native Linux execution remains part of the formal and remote gate;
  the local Linux result is compilation evidence only.
- [x] Private production-helper evidence proves the exact 16,384-byte
  serialized-result guard because every public success payload is smaller.

## Exact cycle-4 local-gate evidence

Exact candidate `78d6fd7e0c42ec97f4f176e8378ab774c25893ca` is locally green
under Rust and Cargo 1.94.1:

- formatting, workspace all-target/all-feature warnings-denied Clippy,
  workspace tests, and workspace doctests pass;
- focused `edit_file` suites pass 43 private production-helper tests, 24 direct
  tests, five engine tests, and seven reference-host tests;
- delivered `write_file` regressions pass 30 private tests, 25 direct tests,
  and five engine tests;
- discovery inventories 684 default-feature tests, 733 all-feature tests, and
  zero benchmarks;
- the repository Python harness passes 130 tests with eight expected macOS
  skips;
- pinned-fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef`
  compatibility is green;
- cargo-deny 0.20.2 and cargo-audit 0.22.2 are green;
- Linux, FreeBSD, and WASI gates pass, while Node actively executes the WASI
  unsupported-target `edit_file` test 1/1;
- documentation integrity covers 62 Markdown files, 437 inline links, and 287
  repository-relative links with zero missing targets;
- the clean base diff passes and contains zero unsafe Rust; and
- a freshly built locked arm64 Mach-O release CLI has SHA-256
  `66d1db86666764b68f79bcb5eb01a6413aab9f27d795a81b27152aa0c24add9d`;
  bare, help, and status smoke paths pass.

The repository harness's earlier 129-test failure was honest: its benchmark
compatibility helper sorted paths while the real Git command supplies source
order. Component `e4eec3cac30ec923c19fa53a81e5b6ba9b81cfae` preserves Git
order and adds the thirtieth benchmark-harness test. The 130-test result and
pinned compatibility result are harness-correctness evidence, not a product-
performance result or compatibility-status promotion.

## Cycle-4 delivery gates

Cycle-4 behavior review is green on exact SHA
`78d6fd7e0c42ec97f4f176e8378ab774c25893ca`. Delivery remains pending until
the documentation seal receives exact feature-branch CI and benchmark
workflows, `main` is fast-forwarded without force, and that exact `main` SHA
receives green CI and benchmark workflows. Documentation-only records are
exempt from a separate adversarial cycle under the user's instruction, but
their applicable remote workflows remain required.

## Pending lineage

- Exact base: `242adfed4be717baf7cd07275aae40ec8a3637f6`
- Contract: `bb0c381f8b044f7849bef80cc482034e1dd57ecf`
- Contract CI: `32634883133`, green
- Contract benchmark evidence: `32634883139`, green
- Local-gate precursor: `31ec79e000589c4fb34599be4aad4f90ea33974f`
- Documentation component after composition:
  `b1210f395a25bc59590c3b4b0164fac56e96bca0`
- Cycle-1 formal-preparation commit:
  `3934d9d26ced78d5164e9ff2620c44ebb6480dd1`
- Failed cycle-1 candidate: `8fdb67892f34a0fbfbb90a54e8eda982159813bf`
- Cycle-1 correctness/API track: green, zero findings
- Cycle-1 filesystem/robustness track: not green, one high and one low finding
- Cycle-1 performance/concurrency track: not green, one high finding
- Production remediation component:
  `578ef3cf2061568d02a160fbe7a498203880b9e9`
- Integrated production remediation:
  `013016f276e023838ffe7ddf8a79121a3ee463a1`
- Independent remediation-test component:
  `59471147817ed7520513fdf51041ec24c822bfe3`
- Exact composed remediation precursor:
  `482d33c0bc586ff594d5b0decc58de347cb9243e`
- Cycle-1 remediation documentation component:
  `b02b4e9c1262042c7f0aa7fc5112520f8c406924`
- Cycle-2 review-preparation commit:
  `5a985798b679e6cfaeeca61af1ced8da42c02bc1`
- Failed cycle-2 candidate: `f84bac87f472fc851eca670764657e5a31ce0256`
- Cycle-2 correctness/API track: green, zero findings
- Cycle-2 filesystem/robustness track: not green, one high and one low finding
- Cycle-2 performance/concurrency track: green, zero findings
- Cycle-2 production remediation component:
  `22197389a521095132c02125726dbe67fbf06d1b`
- Integrated cycle-2 production remediation:
  `7900d97269341a9b8a46bcdcdb987279bc168e4d`
- Cycle-2 independent remediation-test component:
  `65d40d99f4e026834a05778029800fa703c9379e`
- Exact composed cycle-2 remediation behavior:
  `ab6841388838384e27e6299151d50bb83d2ec46e`
- Exact cycle-3 behavior candidate:
  `da1537b229393007101264cd7bc8fd12ee393a3d`
- Cycle-3 correctness/API track: green, zero findings
- Cycle-3 filesystem/robustness track: not green, four low evidence findings;
  no production atomicity defect
- Cycle-3 performance/concurrency track: green, zero findings
- Cycle-4 production remediation component:
  `985b232731883f7a5c18f8f7cbba56dbedfc7c6e`
- Integrated cycle-4 production remediation:
  `1b9ffca9031c61625420279569670c1c80d2d750`
- Cycle-4 independent remediation-test component:
  `3cbb8956477af30cf5f8d63f118e597793267efc`
- Exact integrated cycle-4 remediation:
  `d0d188b39290a50f7f10d7e4665cf694abdfc460`
- Cycle-4 remediation documentation component:
  `590ad386ca7c783d9fd463ab48531fa81025f11c`
- Cycle-4 review-marker parent:
  `12470d9e6c4c9301a0eeaef34e01a1ab31c84d07`
- Exact cycle-4 candidate:
  `78d6fd7e0c42ec97f4f176e8378ab774c25893ca`, tree-identical to its parent
- Cycle-4 correctness/API track: green, zero findings
- Cycle-4 filesystem/robustness track: green, zero findings
- Cycle-4 performance/concurrency track: green, zero findings
- Behavior-green SHA: `78d6fd7e0c42ec97f4f176e8378ab774c25893ca`
- Documentation seal: pending
- Exact feature CI and benchmark evidence: pending
- No-force fast-forward `main`: pending
- Exact `main` CI and benchmark evidence: pending

The cycle-4 candidate is behavior-green but establishes no feature-delivery,
`main`, compatibility-status promotion, equivalence, or product-performance
approval. Zig remains solely the pinned upstream fx benchmark build input; the
machine-god product implementation is Rust.
