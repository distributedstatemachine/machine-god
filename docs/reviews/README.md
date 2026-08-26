# Adversarial reviews

Current bounded slice 32 remains **IN PROGRESS**. Cycle 4 rejected exact
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
current cycle-8 candidate contains this wording correction. Only formal cycle-8
review, remote workflows, `main` integration, and delivery are pending. The
slice remains non-equivalent, unmeasured, and claim-ineligible; no product-
performance or fx-equivalence claim is made.

- [`m03-session-cli-review-01.md`](m03-session-cli-review-01.md) tracks the
  in-progress bounded slice-32 `session <id> [--json]` composition from exact
  delivered base `6e687b6`. It retains the full rejected cycle-4, remediated
  gate-record, rejected cycle-5 and cycle-6 evidence, and rejected cycle-7
  status summarized above. The current cycle-8 candidate contains the wording
  correction; only its formal review and delivery gates remain pending.
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
- [Milestone 03 `permissions` CLI review ledger](m03-permissions-cli-review-01.md)
- [Milestone 03 `models` CLI delivery review ledger](m03-models-cli-review-01.md)
- [Milestone 03 `doctor` CLI live review ledger](m03-doctor-cli-review-01.md)
