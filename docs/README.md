# Documentation

This directory separates durable contracts from mutable delivery state. The
single source of truth for what is current, complete, or next is the
[current delivery state](implementation-plan.md#current-delivery-state).
Overview pages describe the system as it exists; they do not repeat candidate
SHAs, workflow IDs, review counts, or slice-by-slice history.

## Start here

- [Implementation plan](implementation-plan.md) — objective, milestones,
  current delivery state, and remaining work.
- [Architecture](architecture.md) — crate ownership, dependency direction,
  data flow, and invariants.
- [Core API](core-api.md) — provider-neutral public contracts and engine
  behavior.
- [Native reference host](native-reference-host.md) — the concrete composed
  provider, store, prompt adapters, roots, and tool catalog.
- [CLI](cli.md) — command grammar and presentation boundaries.
- [Security](security.md) — authority, trust, confinement, redaction, and
  lifecycle boundaries for ordinary coding-agent operation.
- [Performance](performance.md) — measurement rules and structural resource
  bounds.
- [Compatibility](compatibility.md) — scenario-based comparison policy and the
  pinned upstream input.

## Native host and provider contracts

- [Configuration](configuration.md)
- [AI Gateway codec](ai-gateway.md)
- [AI Gateway HTTP transport](ai-gateway-http.md)
- [AI Gateway credential discovery](ai-gateway-credentials.md)
- [Ask permission adapter](ask-permission.md)
- [Native root selection](native-root-selection.md)
- [Session store](session-store.md)
- [Native session lifecycle](native-session-lifecycle.md)
- [Native session listing](native-session-listing.md)
- [Native session inspection](native-session-inspection.md)
- [Testkit](testkit.md)

## Native tool contracts

- Workspace reading and search: [read_file](read-file.md),
  [list_files](list-files.md), [file_info](file-info.md),
  [glob_files](glob-files.md), and [grep_files](grep-files.md).
- Workspace mutation: [write_file](write-file.md), [edit_file](edit-file.md),
  [delete_file](delete-file.md), [rename_file](rename-file.md),
  [copy_file](copy-file.md), and [create_folder](create-folder.md).
- Host interaction: [open_file](open-file.md), [terminal](terminal.md), and
  [ask_user_question](ask-user-question.md).
- Session-result paging: [read_tool_result](read-tool-result.md).
- Network tools: [web_fetch](web-fetch.md) and
  [web_search](web-search.md).

Each contract owns its public input, authority, result, limit, cancellation,
platform, race, and deferred-scope semantics. Use public Rust constants and the
contract together when exact numeric limits matter.

## CLI contracts

- [permissions](permissions-cli.md)
- [models](models-cli.md)
- [doctor](doctor-cli.md)
- [sessions](sessions-cli.md)
- [session](session-cli.md)

## Decisions and historical evidence

- [Architecture decisions](decisions/README.md)
- [Adversarial review archive](reviews/README.md)

Review ledgers are historical evidence for immutable candidates and finding
dispositions. They are not live status pages. Git history retains detail
removed during periodic documentation compaction.
