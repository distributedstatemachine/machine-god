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

- `/mcp` and `/mcp list` show configured server names, stdio/HTTP transport,
  enabled status and required status. These are explicitly **configured, not
  connected** observations, not a health check or executable catalog.
- `/mcp path` shows the selected native `mcp.json` path with terminal controls
  escaped. It does not create a profile.
- `/mcp add NAME COMMAND [ARGS...]` saves or replaces a stdio configuration.
  Whitespace separates literal tokens; no shell evaluation or quote grouping
  occurs. Replacement of an existing alias follows the pinned add behavior.
- `/mcp remove NAME` removes the exact configured alias.
- `/mcp reload` requests an all-selected runtime replacement through the actual
  host controller. Failure preserves the previously active generation.
- The seven `/mcp resource` and `/mcp prompt` actions use the actual selected
  native runtime and return bounded, observed external data.
- `/mcp auth NAME` requests explicit confirmation with `--open`, without OAuth
  network, credential loading or browser effects. `/mcp auth NAME --open` uses
  the selected native profile, approved browser owner and local callback, saves
  credentials, then reports a separate configured activation receipt.
- `/mcp logout NAME` reports local credential removal and remote revocation
  independently. It does not implicitly reload the runtime.

Mutation receipts separately report confirmed or ambiguous save durability,
changed or unchanged configuration, and runtime activation not attempted.
Ambiguous publication advises inspection and no automatic retry. A configuration
save does not establish a successful connection or reload. Executable arguments,
environment values, remote URLs, headers and credentials are never included in
configuration-list receipts.

Reload and resource/prompt actions require selected runtime authority separately
from profile-management authority. Authentication/logout require the selected
native authorization service; browser authorization additionally requires the
retained desktop launcher and explicit `--open` consent. Missing authority reports the native unavailable error
without effects, prompt fallback or automatic retry.

## Native runtime controls

The native interactive owner dispatches reload and all seven resource/prompt
actions separately from profile management. These operations use only the actual
host controller/runtime; absent runtime authority remains unavailable, without
prompt fallback. Configuration saves still do not activate peers.

On first poll each runtime control acquires the exact accepted conversation's
file-control lifecycle permit and retains it until the operation returns its
receipt. Reload uses configured startup without an aggregate timeout: every peer
attempt retains its full configured budget and caller cancellation remains live.
Profile loading, exact-source validation and local cleanup have separate finite
housekeeping windows on the controller's selected clock. Profile work uses the
controller's retained workers; network/peer futures
are polled asynchronously by the interactive owner, never blocked on a worker.
An accepted reload returns the actual typed controller publication or failure
receipt even when cancellation races publication; it is not automatically retried.

Authentication can select a disabled or failed remote configuration. The real
conversation admission, exact profile, original cancellation and native worker
custody remain retained through credential publication. Successful authorization
then attempts configured activation; failed activation does not undo a confirmed
credential save. Populated logout revokes the refresh and access tokens separately
when supported. An unsuccessful remote revocation remains ambiguous even after
confirmed local removal. Authorization URLs and credentials never enter command
receipts or recorded output. The [authorization contract](mcp-auth.md) owns the
browser, callback, persistence and cleanup details.

Feature commands pass through the existing bounded request conversion and select
an exact native human-command lifetime, not a fabricated model turn. Successful
receipts retain that owner, its cancellation token and the original bounded
feature result through presentation. Borrowed data access does not assert current
authority; `revalidate` checks the original selection, never a replacement.
Protocol failures and unresolved input-required handoffs are failed/incomplete
actions, not permission to continue. Resource and prompt data are not enqueued as
model input or published through a fabricated model archive context.

## Presentation bounds

The global slash envelope remains 64 KiB even though the lower-level native MCP
parser has a larger raw-input limit. Invalid syntax and oversized commands are
rejected locally, not submitted as model prompts. Pending control receipts keep
the existing busy admission behavior.

Rendering uses the shared 64 KiB bounded output. Configured metadata admits at
most 64 rows and 128 bytes per nonempty
name. Path conversion is preceded by a raw-byte check of 4096 directory bytes
plus `/mcp.json`. Names and paths use terminal-safe escaping. Save and error
messages expose only fixed native receipt categories, not configuration bodies.

Feature presentation retains one exact native result and an acknowledged paging
cursor. Each frame borrows at most 8 KiB of raw JSON on UTF-8 boundaries before
terminal escaping and stays within the 64 KiB output bound. The cursor advances
only after flush acknowledgement and transfers unchanged through shutdown
presentation. There is no cloned full-result JSON or queue of rendered pages.
Blocked writes cannot replay a feature request or an acknowledged page.

External descriptor and response text is labelled observed data, never enqueued
as a model prompt. Protocol failures expose only a fixed category and numeric
code, not the peer's error message or data. Unresolved input reports required
interaction and no automatic retry. Reload rendering separately reports
publication/unchanged/closed-after-publication, per-server startup facts and
local cleanup observations; it does not claim remote revocation or infer runtime
success from a configuration save.

These controls do not grant model tool permissions; OAuth effects remain native.
The
[implementation plan](implementation-plan.md) remains the live delivery ledger.
