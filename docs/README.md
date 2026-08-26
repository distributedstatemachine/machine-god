# Documentation

Current bounded Milestone 03 slice 33, native `web_search`, is **IN PROGRESS**
from exact delivered base `4ba9f5afde89b9666fe9929bb81fbabcaa834334`.
Its frozen strict input is required `query` plus optional mutually exclusive
`allowed_domains` / `blocked_domains`. Effect-free preflight prepares the exact
configured AI Gateway `NetworkTarget`; one allowed execution makes at most one
required Perplexity worker request through the shared injected transport, and a
separate bounded native codec admits its one provider-executed call/result.
Production, independent evidence, and these docs are in initial composition.
No composed exact SHA has passed the complete gate or fresh review, so the
slice is not green, integrated, or delivered and the delivered count remains
32. The fixed resource/platform/deferred boundary is the
[`native web search contract`](web-search.md), and status is tracked in its
[`live review ledger`](reviews/m03-web-search-review-01.md). The slice is
unmeasured and makes no product-performance or fx-equivalence claim.

Current bounded Milestone 03 slice 32, strict `session <id> [--json]`, is
**DELIVERED**. Cycle 4 rejected exact `df72e084`, tree `99bf524`, with
correctness/API, native effects, and performance/resources each at `0/0/1/0`;
its deduplicated `0/0/2/0` union is the ordinary/streamed wire-form mismatch
plus eager approximately 8.9 MB
tracker allocation. Exact remediation `1f96c4bf`, tree `b320f552`, makes
`StoredEnvelope`, `StoredRecord`, `StoredMessage`, `StoredToolCall`, and
`StoredToolOutput` object-only and `Role` string-only, preserves the canonical
writer, and grows fixed-fingerprint tracker storage fallibly with unique keys,
with at most 65,536 tracker entries. Exact gate-record candidate `8f533cde`, tree
`8215fb94`, passed the complete exact-1.94.1 local gate without fallback:
focused 24 native/64 CLI process/16 differential, Python 135/8 skips, byte-
stable pinned fx `b1774fb`, WASI/FreeBSD with only the established `read_file`
warning, documentation 85/147/626/81 checks,
`cargo-deny` 0.20.2 with three established duplicate warnings, `cargo-audit`
0.22.2 over 211 dependencies/1,226 advisories with zero vulnerabilities, the
unchanged 364-line production graph, and diff/inventory/no-added-unsafe checks
are green. Its 3,985,216-byte release binary has SHA-256
`c0e83dbfdfba7c4843a1af4c3689bda568045c84dc87ef4d6098cc7a4cd6975c` and
passed the ledger's 16-category/20-record, 12-grammar, missing/no-create,
held-lock, engine-over-default, and 8,650,857-byte near-cap matrix; the native
near-cap probe passed 1/1. Allocator
total/current/maximum tuples are `12/2/7` and `819/14/645` bytes for empty/
short/long text, `14/2/8` and `1,427/14/1,059` for short/long JSON, and
`35/2/9` and `2,228,435/14/1,606,083` for 5,000 keys.

Cycle 5 rejected exact `8f533cde`: correctness/API `0/0/0/1`, native effects
`0/0/0/0`, and performance/resources `0/0/0/1`, deduplicated to one low stale
cross-document-summary finding. That documentation remediation is already
composed in exact cycle-6 candidate
`5332d6a841521f3aa3c26b7c2b9a0e77cb1f7e31`, tree
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
slice stays non-equivalent, unmeasured, and claim-ineligible; no
product-performance or fx-equivalence claim is made. See the
[`live ledger`](reviews/m03-session-cli-review-01.md).

- [Implementation plan](implementation-plan.md)
- [Architecture](architecture.md)
- [Provider-neutral core API](core-api.md)
- [Deterministic testkit](testkit.md)
- [Command-line interface](cli.md)
- [Delivered top-level `session` CLI contract](session-cli.md)
- [Delivered top-level `doctor` CLI contract](doctor-cli.md)
- [Delivered top-level `models` CLI contract](models-cli.md)
- [Delivered top-level `permissions` CLI contract](permissions-cli.md)
- [Delivered top-level `sessions` CLI contract](sessions-cli.md)
- [Native configuration and schema-v3 credential source](configuration.md)
- [Native `read_file` tool](read-file.md)
- [Native `list_files` tool](list-files.md)
- [Native `file_info` tool](file-info.md)
- [Native `glob_files` tool](glob-files.md)
- [Native `grep_files` tool](grep-files.md)
- [Native `write_file` delivered contract](write-file.md)
- [Native `edit_file` tool](edit-file.md)
- [Native `delete_file` tool](delete-file.md)
- [Native `rename_file` tool](rename-file.md)
- [Native `copy_file` contract](copy-file.md)
- [Native `create_folder` delivered contract](create-folder.md)
- [Native `open_file` delivered contract](open-file.md)
- [Native `web_fetch` delivered contract](web-fetch.md)
- [Native `web_search` slice-33 contract](web-search.md)
- [Injected-transport AI Gateway provider](ai-gateway.md)
- [Optional native AI Gateway HTTP transport](ai-gateway-http.md)
- [Native AI Gateway credential discovery](ai-gateway-credentials.md)
- [Native file session store](session-store.md)
- [Native ask permission handler](ask-permission.md)
- [Native reference-host composition](native-reference-host.md)
- [Native root selection and preparation](native-root-selection.md)
- [Native by-ID session lifecycle](native-session-lifecycle.md)
- [Native session listing](native-session-listing.md)
- [Delivered native session inspection](native-session-inspection.md)
- [Compatibility](compatibility.md)
- [Performance](performance.md)
- [Security](security.md)
- [Architecture decisions](decisions/README.md)
- [Adversarial reviews](reviews/README.md)
- [Milestone 01 completion evidence](reviews/m01-milestone-review.md)
- [Milestone 02 completion evidence](reviews/m02-milestone-review.md)
- [Milestone 03 config/status CLI delivery review](reviews/m03-config-status-cli-review-01.md)
- [Milestone 03 native host configuration schema-v2 review](reviews/m03-native-host-config-review-01.md)
- [Milestone 03 native reference-host composition review](reviews/m03-native-reference-host-review-01.md)
- [Milestone 03 configured credential-source review](reviews/m03-configured-credential-source-review-01.md)
- [Milestone 03 native root-selection delivery review](reviews/m03-native-root-selection-review-01.md)
- [Milestone 03 native session-lifecycle review](reviews/m03-native-session-lifecycle-review-01.md)
- [Milestone 03 native session-listing delivery review](reviews/m03-native-session-listing-review-01.md)
- [Milestone 03 native `file_info` delivery review](reviews/m03-file-info-review-01.md)
- [Milestone 03 native `glob_files` delivery review](reviews/m03-glob-files-review-01.md)
- [Milestone 03 native `grep_files` delivery review](reviews/m03-grep-files-review-01.md)
- [Milestone 03 native `write_file` delivery review](reviews/m03-write-file-review-01.md)
- [Milestone 03 native `edit_file` delivery review](reviews/m03-edit-file-review-01.md)
- [Milestone 03 native `delete_file` delivery review](reviews/m03-delete-file-review-01.md)
- [Milestone 03 native `rename_file` delivery review](reviews/m03-rename-file-review-01.md)
- [Milestone 03 native `copy_file` delivery review](reviews/m03-copy-file-review-01.md)
- [Milestone 03 native `create_folder` delivery review](reviews/m03-create-folder-review-01.md)
- [Milestone 03 native `open_file` delivery review](reviews/m03-open-file-review-01.md)
- [Milestone 03 native `web_fetch` review ledger](reviews/m03-web-fetch-review-01.md)
- [Milestone 03 native `web_search` live review ledger](reviews/m03-web-search-review-01.md)
- [Milestone 03 `permissions` CLI review ledger](reviews/m03-permissions-cli-review-01.md)
- [Milestone 03 `models` CLI delivery review ledger](reviews/m03-models-cli-review-01.md)
- [Milestone 03 `doctor` CLI live review ledger](reviews/m03-doctor-cli-review-01.md)
- [Milestone 03 `sessions` CLI live review ledger](reviews/m03-sessions-cli-review-01.md)
- [Milestone 03 `session` CLI live review ledger](reviews/m03-session-cli-review-01.md)
