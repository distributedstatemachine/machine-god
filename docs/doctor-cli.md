# Native `doctor` CLI contract

The comparison observation remains pinned to `vercel-labs/fx` commit
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Machine-god deliberately does
not copy fx's complete diagnostic inventory, report schema, remediation text,
or exit semantics. This bounded command has exactly four machine-god checks
and the output below. It is implemented but non-equivalent, not measured, and
claim-ineligible in bootstrap evidence. This command makes no compatibility,
fx-equivalence, latency, throughput, memory, size-improvement, or other product-
performance claim.

## Command grammar

The only accepted forms are:

```text
machine-god doctor
machine-god doctor --json
```

`--json` is accepted exactly once and only after `doctor`. Unknown, reordered,
repeated, additional, or non-Unicode arguments fail before any check: exit 2,
empty stdout, and the global invalid-argument diagnostic with final LF:

```text
machine-god: invalid arguments
Usage: machine-god [help | --help | -h | --version | -V | doctor [--json] | models [--json] | permissions [--json] | status [--json]]
```

Global help retains `help` first and inserts `doctor` before `models`. Its exact
row is:

```text
  doctor       Run local health and preflight checks
```

There is no command-local help form and no interactive `/doctor` command.

## Check order and classification

A valid invocation always evaluates and reports exactly these four checks in
this order: `config`, `credential`, `state`, `platform`. A diagnostic `fail`
does not short-circuit later checks and is report data, not command failure.
Each status is exactly `ok`, `warn`, or `fail`.

Native owns the authoritative check/status/credential-status value types,
process and injected inspection, classification, order, and counts. The CLI
owns no diagnostic truth: after parsing, it takes the native report, validates
the exact four-check/count invariant into an internal render snapshot, and owns
only bounded human/JSON rendering plus channel and exit handling. Core remains
unchanged and receives no environment, filesystem, credential, or platform
authority.

| Check | Condition | Status | Exact detail |
| --- | --- | --- | --- |
| `config` | A strict supported file was loaded. | `ok` | `configuration file is valid` |
| `config` | The file or configuration location is absent, so built-in defaults apply. | `warn` | `configuration file is missing; using built-in defaults` |
| `config` | Selected environment is invalid. | `fail` | `native configuration environment is invalid` |
| `config` | Selected path is not a regular file. | `fail` | `native configuration path is not a regular file` |
| `config` | File cannot be safely inspected or read. | `fail` | `native configuration file is unreadable` |
| `config` | File exceeds the existing bound. | `fail` | `native configuration file is too large` |
| `config` | Strict content is invalid. | `fail` | `native configuration format is invalid` |
| `config` | Schema version is unsupported. | `fail` | `native configuration schema version is unsupported` |
| `credential` | The selected valid source is OIDC. | `ok` | `VERCEL_OIDC_TOKEN is configured` |
| `credential` | The selected valid source is the API key. | `ok` | `AI_GATEWAY_API_KEY is configured` |
| `credential` | Both named values are absent or empty. | `fail` | `no AI Gateway credential is configured` |
| `credential` | The selected environment value is invalid. | `fail` | `AI Gateway credential environment is invalid` |
| `credential` | The selected value is not an accepted bearer token. | `fail` | `AI Gateway bearer token is invalid` |
| `credential` | Credential inspection is unavailable in this build. | `fail` | `credential inspection is unavailable on this build` |
| `state` | The selected state path is a directory. | `ok` | `state directory is ready` |
| `state` | The selected state directory does not exist. | `warn` | `state directory is not initialized` |
| `state` | The selected path exists but is not a directory. | `fail` | `state path is not a directory` |
| `state` | Metadata inspection failed. | `fail` | `state directory is inaccessible` |
| `state` | No state location can be selected. | `fail` | `state directory location is unavailable` |
| `state` | Selected state environment is invalid. | `fail` | `state directory environment is invalid` |
| `platform` | Target is Linux or macOS. | `ok` | `native host platform is supported` |
| `platform` | Any other target. | `fail` | `native host platform is unsupported` |

Configuration preserves the existing strict v1/v2/v3 loader, 64 KiB retained-
input cap plus overflow witness, safe built-in defaults, supported-Unix no-
follow opening, and fixed redacted `NativeConfigError` display strings. It
does not reveal the loaded schema, permission mode, provider, transport, model,
credential-source field, configuration path, or file content.

Credential inspection preserves the existing fixed precedence: a nonempty
`VERCEL_OIDC_TOKEN` is selected before a nonempty `AI_GATEWAY_API_KEY`; an
invalid selected higher-priority value fails without fallback. Output names
only the selected non-secret environment-variable name and never a token,
length, fragment, or arbitrary environment value. Missing credentials are a
`fail`, including when public model-catalog access would be possible, because
this check reports generation-host readiness.

State inspection uses the existing XDG/HOME selection and metadata-only state
classification. A missing directory is `warn`; doctor never prepares it. The
platform check is compile-time target classification. Linux and macOS are the
only supported native-host platforms for this command; another target still
receives a complete four-check report whose platform check is `fail`.

## Exact output

Human output begins with one summary and then exactly four check lines:

```text
[doctor] ok=N warn=N fail=N
[<status>] config: <detail>
[<status>] credential: <detail>
[<status>] state: <detail>
[<status>] platform: <detail>
```

JSON is one compact object with fixed top-level key order
`kind,ok_count,warn_count,fail_count,checks`. Every check object has fixed key
order `name,status,detail`. For a supported host with missing config, missing
credential, and missing state roots, it is exactly:

```json
{"kind":"doctor","ok_count":1,"warn_count":2,"fail_count":1,"checks":[{"name":"config","status":"warn","detail":"configuration file is missing; using built-in defaults"},{"name":"credential","status":"fail","detail":"no AI Gateway credential is configured"},{"name":"state","status":"warn","detail":"state directory is not initialized"},{"name":"platform","status":"ok","detail":"native host platform is supported"}]}
```

Both representations end in one LF. Each count is in `0..=4`, their checked
sum is exactly four, and each equals the number of checks bearing that status.
The complete serialized representation, including the final LF, is built under
an inclusive 4,096-byte cap before its first stdout write. Names, statuses, and
details are closed fixed strings; output includes no path or arbitrary external
data.

Any completed report, including one or more `fail` checks, exits 0 and writes
nothing to stderr. Failure to render the bounded report exits 1, writes no
success report, and uses exactly:

```text
machine-god doctor: could not render report
```

Failure while writing the already-rendered report exits 1 and preserves the
existing fixed diagnostic:

```text
machine-god: failed to write output
```

No partial write is claimed as success. Both diagnostics end in one LF.

## Authority and side effects

Doctor is a synchronous, read-only local projection. It may inspect only the
selected configuration file, the two fixed credential environment values, the
selected state-directory metadata, and compile-time platform support. It does
not create a config or state root, repair permissions, migrate or rewrite a
file, construct or poll an engine/provider/transport, contact a network, spawn
a process, start a runtime, open a session or workspace, select or list a model,
prompt, persist, cache, or mutate any product state.

Output contains no network, process, runtime, session, workspace, model, or
path state. Fixed diagnostic details reveal neither secret bytes nor operating-
system error text. The command owns no cancellation or signal behavior because
its bounded synchronous checks start only after complete parsing and spawn no
background work.

## Required evidence

Independent evidence must cover:

- exact grammar, parse-before-effects, help/usage ordering, non-Unicode input,
  exit codes, streams, JSON key order, escaping, and final LF;
- exactly four ordered checks, every fixed detail/status mapping, count/check
  consistency, report-failure exit 0, and both exact exit-1 diagnostics;
- valid v1/v2/v3, missing/default, invalid environment/type/read/size/format/
  version configuration cases without path or content reflection;
- both credential sources, precedence, empty/missing, non-Unicode, malformed,
  oversized, and unavailable-build cases without secret reflection;
- every state classification, Linux/macOS support, other-platform failure, and
  no platform detail derived from runtime or ambient process state;
- inclusive 4,096-byte rendering and failure before the first success write;
- no configuration rewrite and no config/state/home root creation; and
- freshly built release-binary human/JSON/count-consistency/no-create smokes on
  isolated missing XDG roots.
