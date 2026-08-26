# Documentation

Current bounded Milestone 03 slice 32, strict `session <id> [--json]`, remains
**IN PROGRESS**. Cycle 4 rejected exact `df72e084`, tree `99bf524`, with
correctness/API, native effects, and performance/resources each at `0/0/1/0`;
its deduplicated `0/0/2/0` union is the ordinary/streamed wire-form mismatch
plus eager approximately 8.9 MB
tracker allocation. Exact remediation `1f96c4bf`, tree `b320f552`, makes
`StoredEnvelope`, `StoredRecord`, `StoredMessage`, `StoredToolCall`, and
`StoredToolOutput` object-only and `Role` string-only, preserves the canonical
writer, and grows fixed-fingerprint tracker storage fallibly with unique keys
under the 65,536-node ceiling. Exact gate-record candidate `8f533cde`, tree
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
cross-document-summary finding. This documentation remediation, three fresh
cycle-6 reviews, remote workflows, `main` integration, and delivery remain
pending. The slice stays non-equivalent, unmeasured, and claim-ineligible; no
product-performance or fx-equivalence claim is made. See the
[`live ledger`](reviews/m03-session-cli-review-01.md).

- [Implementation plan](implementation-plan.md)
- [Architecture](architecture.md)
- [Provider-neutral core API](core-api.md)
- [Deterministic testkit](testkit.md)
- [Command-line interface](cli.md)
- [Cycle-5-rejected top-level `session` CLI contract](session-cli.md)
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
- [Injected-transport AI Gateway provider](ai-gateway.md)
- [Optional native AI Gateway HTTP transport](ai-gateway-http.md)
- [Native AI Gateway credential discovery](ai-gateway-credentials.md)
- [Native file session store](session-store.md)
- [Native ask permission handler](ask-permission.md)
- [Native reference-host composition](native-reference-host.md)
- [Native root selection and preparation](native-root-selection.md)
- [Native by-ID session lifecycle](native-session-lifecycle.md)
- [Native session listing](native-session-listing.md)
- [Cycle-5-rejected native session inspection (cycle 6 pending)](native-session-inspection.md)
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
- [Milestone 03 `permissions` CLI review ledger](reviews/m03-permissions-cli-review-01.md)
- [Milestone 03 `models` CLI delivery review ledger](reviews/m03-models-cli-review-01.md)
- [Milestone 03 `doctor` CLI live review ledger](reviews/m03-doctor-cli-review-01.md)
- [Milestone 03 `sessions` CLI live review ledger](reviews/m03-sessions-cli-review-01.md)
- [Milestone 03 `session` CLI live review ledger](reviews/m03-session-cli-review-01.md)
