# Adversarial reviews

Bounded slice 35, native `ask_user_question`, is **CYCLE 3 REJECTED — CYCLE 4
REMEDIATION IN PROGRESS** from exact delivered base `5846799`. It is limited to
ordinary questions through an injected rootless prompter: strict 1-4/2-6 input,
terminal-safe bounded text,
bounded ordered free-form answers, explicit no-policy-authority preparation,
separately owned prompter interaction authority, fixed
redacted outcomes, and default-one/hard-eight fail-fast prompt concurrency.
`permission_request_id`, CLI presentation, timeout, detached work, approval
escalation, and later-tool scope are deferred. Cycle-2 production, independent
evidence, and its exact local candidate gate were completed, but three formal
tracks rejected exact candidate `910d7bc`, tree `503a91f`, with a deduplicated
`0/0/2/2` union. Cycle-3 source `cf531d1`/`b7b4358`, evidence
`3e3c0c7`/`f3f6f9d`, and docs `bfdf05b` compose at exact behavior head
`8bdc33d96bf88f5986c0e01b3979a2cef0427e82`, tree
`7a342fc27d6b2d65dcbdcf547cfbdc8214e73702`. The complete exact-1.94.1
local gate is green with 57 focused tests. Formal cycle 3 rejected exact
candidate `746e510c7d8eb93229996e74f91827f489e5bb31`, tree
`c49221efbea66c840b333f0de0161aa686aad52f`: correctness/API reported
`0/0/1/2`, lifecycle/platform `0/0/0/2`, performance/resources `0/0/2/0`, and
the deduplicated union is `0/0/3/2`. Cycle-4 remediation, its replacement gate,
three fresh tracks, remote workflows, and delivery remain pending. See
[`m03-ask-user-question-review-01.md`](m03-ask-user-question-review-01.md).

Bounded slice 34, native `terminal`, is **DELIVERED** from exact delivered base
`52b5885`. It implements only bounded
foreground `exec` with a Linux-only system executor. Formal cycle 5 rejected
exact candidate `0c04859`, tree `b0359fb`, with a deduplicated `0/1/0/1`
union: a high self-target notifier cycle and a low stale-status defect. Cycle-6
source, independent evidence, and maintained documentation were completed in
non-overlapping worktrees and compose through exact behavior head `8705811`,
tree `8a319c1`. Its complete exact-1.94.1 replacement gate is green. The
immutable candidate was `28292b7`, tree `487f160`. Formal cycle 6 rejected it:
correctness/API reported `0/0/1/0`, while lifecycle/platform and performance/
resources each reported `0/0/0/0`. The sole medium is stale host-Waker delivery
after pending outer-future drop. Cycle-7 source, independent evidence, and
maintained documentation compose through exact behavior head `9810ee9`, tree
`cf7390e`, whose complete exact-1.94.1 replacement gate is green. The immutable
candidate was `3f07389`, tree `f048cdc`. Formal cycle 7 is **GREEN**:
correctness/API, lifecycle/platform, and performance/resources each reported
`0/0/0/0`. Review-exempt delivered record `ddd6a89`, tree `4a227b3`, passed
feature CI/Benchmark runs `33040148977`/`33040148920` and main CI/Benchmark
runs `33040487021`/`33040487046`; each benchmark run retained exactly two
unexpired exact-SHA artifacts. The delivered count is 34. Scope and worktree
cleanup are recorded in
[`m03-terminal-review-01.md`](m03-terminal-review-01.md).

Bounded slice 33, native `web_search`, is **DELIVERED** from exact delivered
base `4ba9f5a`. Its frozen
local-tool contract performs exact Gateway-network preflight, then at most one
approved required Perplexity worker request through the shared transport and a
dedicated bounded native provider-executed decoder. Composed behavior precursor
`3d298400`, tree `5222c3e`, passed the complete exact-1.94.1 local gate. Formal
cycle 1 rejected exact `89c5ec95`, tree `8d91a55`; the three tracks reported
`0/1/3/2`, `0/2/3/1`, and `0/0/3/0`, deduplicated to `0/2/5/2` in
blocker/high/medium/low order. Source remediation is composed from exact
isolated components `096b11c4` and `ca0b990a`. Exact composed precursor
`e662fa8`, tree `6c0ace9`, passes the complete local gate. Cycle 2's accepted
`0/1/1/1` union was remediated through exact precursor `366cef9`, tree
`40c05cb`. Formal cycle 3 rejected exact candidate `aef6abe`, tree `5abcef3`,
with a deduplicated `1/0/2/2`. Exact isolated components `5d45dca` and
`454f8fd` compose its remediation. Exact precursor `b834205`, tree `f3557a5`,
passes the complete replacement gate. Formal cycle 4 rejected exact `cc1d3d1`,
tree `ad0c3d3`, with a deduplicated `0/0/1/1`; the remediation passed formal
cycle 5 with a `0/0/0/0` union. Review-exempt delivery record `52b5885`, tree
`148b358`, passed feature CI/Benchmark runs `33023313461`/`33023313463` and
main CI/Benchmark runs `33023812814`/`33023812808`; every run succeeded for
the exact SHA, and both benchmark runs retained two unexpired exact-SHA
artifacts. The delivered count is 33. Review scope, fixed resources, deferrals,
and the per-iteration committed/
integrated/clean worktree-removal invariant are tracked in
[`m03-web-search-review-01.md`](m03-web-search-review-01.md). No performance or
fx-equivalence claim is made.

Current bounded slice 32 is **DELIVERED**. Cycle 4 rejected exact
`df72e084`, tree `99bf524`, with correctness/API, native effects, and
performance/resources each at `0/0/1/0`; the deduplicated `0/0/2/0` union is
the wire-form mismatch plus eager approximately 8.9 MB tracker allocation.
Exact remediation `1f96c4bf`, tree `b320f552`, makes `StoredEnvelope`,
`StoredRecord`, `StoredMessage`, `StoredToolCall`, and `StoredToolOutput`
object-only and `Role` string-only, keeps the canonical writer unchanged, and
grows fixed-fingerprint tracker storage fallibly with unique keys, with at most
65,536 tracker entries. Exact gate-record candidate `8f533cde`, tree `8215fb94`,
passed the complete exact-1.94.1 local gate without fallback: focused 24
native/64 CLI process/16 differential, Python 135/8 skips, byte-stable pinned
fx `b1774fb`, WASI/FreeBSD with only the established `read_file` warning, docs
85/147/626/81, `cargo-deny` 0.20.2 with three established duplicate warnings,
`cargo-audit` 0.22.2 over 211 dependencies/
1,226 advisories with zero vulnerabilities, the unchanged 364-line production
graph, and diff/inventory/no-added-unsafe evidence are green.

The 3,985,216-byte release binary has SHA-256
`c0e83dbfdfba7c4843a1af4c3689bda568045c84dc87ef4d6098cc7a4cd6975c` and
passed 16 equivalence categories across 20 records, 12 grammar cases, missing/
no-create, held-lock, engine-over-default, and 8,650,857-byte near-cap evidence;
the native near-cap probe passed 1/1. Allocator
total/current/maximum tuples are `12/2/7` and `819/14/645` bytes for empty/
short/long text, `14/2/8` and `1,427/14/1,059` for short/long JSON, and
`35/2/9` and `2,228,435/14/1,606,083` for 5,000 keys. Cycle 5 rejected exact
`8f533cde`: correctness/API `0/0/0/1`, native effects `0/0/0/0`, and
performance/resources `0/0/0/1`, deduplicated to one low stale cross-document-
summary finding. That documentation remediation is already composed in exact
cycle-6 candidate `5332d6a841521f3aa3c26b7c2b9a0e77cb1f7e31`, tree
`d2fec0815b60c61368298e7f4f0d7bef0fc2e097`. Formal cycle 6 rejected it:
correctness/API, native effects, and performance/resources each reported
`0/0/0/1`; the deduplicated `0/0/0/1` is solely that these pages described the
committed remediation as pending. There was no additional production, API,
native, or performance finding. Formal cycle 7 rejected exact
`399e75eda0f61501fe179a22de6a0f4f2abfce06`, tree
`d056b96ef8361e841c936c5f61c138de913b5fff`: correctness/API and native effects
each reported `0/0/0/0`, while performance/resources reported `0/0/0/1`; the
deduplicated union is `0/0/0/1`. The sole low corrects resource wording:
shadowed duplicate values may parse more nodes than survive in the final tree.
The 65,536 caps apply separately to tracker entries and aggregate final decoded-
tree logical-node accounting, while the 8,651,165-byte file ceiling bounds
total parse work. Production and resource behavior were otherwise green. The
wording correction is present. Formal cycle 8 is **GREEN** on exact reviewed
candidate `d724b6195324349cc5628a47f8ab7fa496123cd5`, tree
`6439863a9b7fd1720156c790fedc4798256c2b6d`: correctness/API, native/effects,
and performance/resources each reported `0/0/0/0`, with a deduplicated
`0/0/0/0` union. Independent evidence included 598 release-binary
differentials with zero mismatches plus focused 24 native, 14 CLI unit, and 24
CLI process tests; native also reconfirmed focused 24 and green WASI/FreeBSD
checks with the established `read_file` warning; performance confirmed the
exact allocator tuples above and the corrected cap semantics. This
documentation-only result seal is review-exempt: it records review of
`d724b61` / `6439863` and does not imply that the seal commit itself was
reviewed. Review-exempt documentation seal
`b6db9a67c070f7ef599d994c44b4a21731a004c5`, tree
`59dd628fd0552c5083449f7a31aa4241a8ecb952`, passed feature CI run
`32965947722` and Benchmark evidence run `32965947723`, was integrated on
`main`, and passed main CI run `32966531225` and Benchmark evidence run
`32966531319`. All four runs succeeded for exact `b6db9a6`; each benchmark
run retained exactly two unexpired exact-SHA artifacts. Slice 32 is delivered,
and the delivered count is 32. No product-performance or fx-equivalence claim
is made. Zig is used only to build the pinned upstream fx comparison input;
`machine-god` remains a Rust product and is neither written in nor shipped as
Zig. This final delivery-record commit is docs-only and user-exempt from
adversarial review. A commit cannot contain its own future workflow IDs, so
the exact workflow IDs for this record will be reported at handoff. The
slice remains non-equivalent, unmeasured, and claim-ineligible; no product-
performance or fx-equivalence claim is made.

- [`m03-session-cli-review-01.md`](m03-session-cli-review-01.md) tracks the
  delivered bounded slice-32 `session <id> [--json]` composition from exact
  delivered base `6e687b6`. It retains the full rejected cycle-4, remediated
  gate-record, rejected cycle-5 through cycle-7 evidence, and formal green
  cycle-8 result summarized above. The review-exempt result seal records the
  reviewed `d724b61` / `6439863` candidate. Exact seal `b6db9a6` completed the
  feature and main delivery gates recorded above.
  Historical initial composition was `852fec7`, focused remediation advanced
  through precursor `c0c16a7`, and gate precursor `fa099f7`, tree `64d6a72`,
  passed its local and supplemental gates before formal cycle-1 review.

- [`m03-sessions-cli-review-01.md`](m03-sessions-cli-review-01.md) tracks the
  delivered bounded slice-31 `sessions [--json]` candidate. Cycle 1 rejected
  exact candidate `9448738` with `0/0/0/3` deduplicated findings. Exact
  replacement `a527652`, tree `0249dd0`, passed three fresh cycle-2 reviews at
  `0/0/0/0` each. Seal `b5b9116`, tree `3e61754`, passed exact feature and main
  CI/benchmark delivery gates.

Each feature and milestone receives fresh correctness/API, security/abuse, and
performance/concurrency reviews. Store reports as `mNN-feature-review-NN.md` and
record the exact reviewed commit, findings, resolutions, and rejected rationales.

- [Milestone 01 completion evidence and review status](m01-milestone-review.md)
- [Milestone 02 core-contracts review](m02-core-contracts-review-01.md)
- [Milestone 02 deterministic testkit review](m02-testkit-review-01.md)
- [Milestone 02 bounded tool-loop review](m02-tool-loop-review-01.md)
- [Milestone 02 completion evidence and review status](m02-milestone-review.md)
- [Milestone 03 config/status CLI delivery review](m03-config-status-cli-review-01.md)
- [Milestone 03 bounded native configuration loading review](m03-native-config-load-review-01.md)
- [Milestone 03 capability-aware tool preflight review](m03-tool-preflight-review-01.md)
- [Milestone 03 confined native read_file review](m03-read-file-review-01.md)
- [Milestone 03 Linux containment marker remediation review](m03-benchmark-containment-marker-review-01.md)
- [Milestone 03 confined native list_files review](m03-list-files-review-01.md)
- [Milestone 03 injected-transport AI Gateway review](m03-ai-gateway-review-01.md)
- [Milestone 03 native AI Gateway HTTP transport review](m03-ai-gateway-http-review-01.md)
- [Milestone 03 native file session store review](m03-session-store-review-01.md)
- [Milestone 03 native ask permission handler review](m03-ask-permission-review-01.md)
- [Milestone 03 native AI Gateway credential discovery review](m03-ai-gateway-credential-review-01.md)
- [Milestone 03 native host configuration schema-v2 review](m03-native-host-config-review-01.md)
- [Milestone 03 native reference-host composition review](m03-native-reference-host-review-01.md)
- [Milestone 03 configured credential-source review](m03-configured-credential-source-review-01.md)
- [Milestone 03 native root-selection delivery review](m03-native-root-selection-review-01.md)
- [Milestone 03 native session-lifecycle review](m03-native-session-lifecycle-review-01.md)
- [Milestone 03 native session-listing delivery review](m03-native-session-listing-review-01.md)
- [Milestone 03 native `file_info` delivery review](m03-file-info-review-01.md)
- [Milestone 03 native `glob_files` delivery review](m03-glob-files-review-01.md)
- [Milestone 03 native `grep_files` delivery review](m03-grep-files-review-01.md)
- [Milestone 03 native `write_file` delivery review](m03-write-file-review-01.md)
- [Milestone 03 native `edit_file` delivery review](m03-edit-file-review-01.md)
- [Milestone 03 native `delete_file` delivery review](m03-delete-file-review-01.md)
- [Milestone 03 native `rename_file` delivery review](m03-rename-file-review-01.md)
- [Milestone 03 native `copy_file` delivery review](m03-copy-file-review-01.md)
- [Milestone 03 native `create_folder` delivery review](m03-create-folder-review-01.md)
- [Milestone 03 native `open_file` delivery review](m03-open-file-review-01.md)
- [Milestone 03 native `web_fetch` review ledger](m03-web-fetch-review-01.md)
- [Milestone 03 native `web_search` live review ledger](m03-web-search-review-01.md)
- [Milestone 03 native `terminal` live review ledger](m03-terminal-review-01.md)
- [Milestone 03 `permissions` CLI review ledger](m03-permissions-cli-review-01.md)
- [Milestone 03 `models` CLI delivery review ledger](m03-models-cli-review-01.md)
- [Milestone 03 `doctor` CLI live review ledger](m03-doctor-cli-review-01.md)
