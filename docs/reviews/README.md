# Adversarial review archive

These ledgers preserve historical evidence about immutable feature candidates.
They are not a second project-status system. Consult the
[current delivery state](../implementation-plan.md#current-delivery-state) for
the active milestone and next work; use Git history for detail removed by
periodic compaction.

## Review policy

Every behavior-changing feature is bound to one exact commit and tree after its
focused and complete local gates pass. Three fresh product reviewers inspect
that same candidate from independent perspectives:

1. correctness, API, and compatibility;
2. lifecycle, platform, and effect boundaries; and
3. performance, concurrency, and resource bounds.

Any confirmed finding rejects the candidate. The finding is fixed, the full
replacement gate is rerun, and three new reviewers inspect a new exact
candidate. Delivery proceeds only after every track is green, the feature
branch workflows pass for the exact SHA, `main` is fast-forwarded without
force, and the exact `main` workflows pass.

A documentation-only result or delivery seal that records already reviewed
behavior is exempt from a redundant product-review cycle. It still requires
the applicable exact-commit checks. Reviews cover ordinary terminal-agent
product behavior; they are not penetration tests.

## Ledger rules

- Record the exact candidate and tree reviewed, the review scopes, findings,
  disposition, and the replacement that closes each accepted finding.
- Keep normative behavior in the subsystem contract, not in its ledger.
- Do not copy live milestone status, delivered counts, or workflow dashboards
  into this index or other overview pages.
- Treat old ledgers as an audit trail. Compact repetitive prose when it stops
  helping implementation, while retaining decisive candidates and finding
  dispositions in Git history.

## Milestone 01

- [Bootstrap](m01-bootstrap-review-01.md)
- [Compatibility inventory](m01-compatibility-review-01.md)
- [Pinned upstream benchmark harness](m01-upstream-benchmark-review-01.md)
- [Milestone review](m01-milestone-review.md)

## Milestone 02

- [Core contracts](m02-core-contracts-review-01.md)
- [Tool loop](m02-tool-loop-review-01.md)
- [Testkit](m02-testkit-review-01.md)
- [Milestone review](m02-milestone-review.md)

## Milestone 03: host foundations

- [Configuration/status CLI](m03-config-status-cli-review-01.md)
- [Native configuration loading](m03-native-config-load-review-01.md)
- [Tool preflight](m03-tool-preflight-review-01.md)
- [AI Gateway codec](m03-ai-gateway-review-01.md)
- [AI Gateway HTTP transport](m03-ai-gateway-http-review-01.md)
- [AI Gateway credentials](m03-ai-gateway-credential-review-01.md)
- [Ask permission](m03-ask-permission-review-01.md)
- [Native host configuration](m03-native-host-config-review-01.md)
- [Configured credential source](m03-configured-credential-source-review-01.md)
- [Native reference host](m03-native-reference-host-review-01.md)
- [Native root selection](m03-native-root-selection-review-01.md)
- [Session store](m03-session-store-review-01.md)
- [Native session lifecycle](m03-native-session-lifecycle-review-01.md)
- [Native session listing](m03-native-session-listing-review-01.md)
- [Benchmark containment marker](m03-benchmark-containment-marker-review-01.md)

## Milestone 03: native tools

- [read_file](m03-read-file-review-01.md)
- [list_files](m03-list-files-review-01.md)
- [file_info](m03-file-info-review-01.md)
- [glob_files](m03-glob-files-review-01.md)
- [grep_files](m03-grep-files-review-01.md)
- [write_file](m03-write-file-review-01.md)
- [edit_file](m03-edit-file-review-01.md)
- [delete_file](m03-delete-file-review-01.md)
- [rename_file](m03-rename-file-review-01.md)
- [copy_file](m03-copy-file-review-01.md)
- [create_folder](m03-create-folder-review-01.md)
- [open_file](m03-open-file-review-01.md)
- [web_fetch](m03-web-fetch-review-01.md)
- [web_search](m03-web-search-review-01.md)
- [terminal](m03-terminal-review-01.md)
- [ask_user_question](m03-ask-user-question-review-01.md)
- [read_tool_result](m03-read-tool-result-review-01.md)

## Milestone 03: CLI surfaces

- [permissions](m03-permissions-cli-review-01.md)
- [models](m03-models-cli-review-01.md)
- [doctor](m03-doctor-cli-review-01.md)
- [sessions](m03-sessions-cli-review-01.md)
- [session](m03-session-cli-review-01.md)
- [resume](m03-resume-cli-review-01.md)

## Milestone 05: extensibility tools

- [semantic_search](m05-semantic-search-review-01.md)
- [memory](m05-memory-review-01.md)
- [skill](m05-skill-review-01.md)
