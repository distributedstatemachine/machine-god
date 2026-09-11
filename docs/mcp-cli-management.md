# MCP profile management in the interactive CLI

The thin interactive CLI recognizes `/mcp` and delegates parsed intent to the
native interactive control owner. It does not load or mutate configuration in
the command parser or presentation layer. The
[native command grammar](mcp-cli.md) defines token and configuration semantics;
the [profile store](mcp-persistence.md) owns observation and publication.

## Startup and authority

Startup takes the parent directory of the selected native configuration file
from its already captured environment. It constructs an explicit MCP store and
management service before acquiring the full terminal host. Construction does
not load `mcp.json`, resolve environment values, connect to servers or create
files. State directories, workspace directories and fx profiles are not fallback
MCP locations. Without a selected native profile, no management service is
installed; malformed selected authority is an error, not an ambient fallback.

Commands that observe configuration report invalid stored configuration rather
than substituting an empty catalog. Path inspection is metadata-only and does
not validate the file contents. The native control owner retains cancellation,
session admission, worker completion and publication receipts; rendering has no
authority to retry or roll back a mutation.

## Available commands and receipts

- `/mcp` and `/mcp list` show configured server names, stdio/HTTP/SSE transport,
  enabled status and required status. These are explicitly **configured, not
  connected** observations, not a health check or executable catalog.
- `/mcp path` shows the selected native `mcp.json` path with terminal controls
  escaped. It does not create a profile.
- `/mcp add NAME COMMAND [ARGS...]` saves or replaces a stdio configuration.
  Whitespace separates literal tokens; no shell evaluation or quote grouping
  occurs. Replacement of an existing alias follows the pinned add behavior.
- `/mcp remove NAME` removes the exact configured alias.

Mutation receipts separately report confirmed or ambiguous save durability,
changed or unchanged configuration, and runtime activation not attempted.
Ambiguous publication advises inspection and no automatic retry. A configuration
save does not establish a successful connection or reload. Executable arguments,
environment values, remote URLs, headers and credentials are never included in
configuration-list receipts.

Reload, authentication/logout and resource/prompt forms remain recognized by the
native grammar but unavailable through this management-only service. They report
the native unavailable error without effects, prompt fallback or automatic retry.
Help advertises usable profile-management forms and explicitly marks those
runtime operations unavailable.

## Presentation bounds

The global slash envelope remains 64 KiB even though the lower-level native MCP
parser has a larger raw-input limit. Invalid syntax and oversized commands are
rejected locally, not submitted as model prompts. Pending control receipts keep
the existing busy admission behavior.

Rendering uses the shared 64 KiB bounded output and returns no partial output on
overflow. Configured metadata admits at most 64 rows and 128 bytes per nonempty
name. Path conversion is preceded by a raw-byte check of 4096 directory bytes
plus `/mcp.json`. Names and paths use terminal-safe escaping. Save and error
messages expose only fixed native receipt categories, not configuration bodies.

These profile controls do not implement transport activation, tool permission
preparation, OAuth, catalog publication or the complete MCP feature. The
[implementation plan](implementation-plan.md) remains the live delivery ledger.
