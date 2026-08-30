# Top-level workspace command

The bounded `workspace` command reports the process's primary workspace as a
read-only lexical snapshot. It does not manage additional directories or
construct the native reference host. Current delivery state and gate evidence
remain only in the
[implementation plan](implementation-plan.md#current-delivery-state); this page
defines durable behavior.

## Grammar and exits

The only accepted invocations are:

```text
machine-god workspace
machine-god workspace --json
machine-god workspace list
machine-god workspace list --json
```

`list` is the default action. The singleton `--json` flag must be final.
`--json list`, repeated flags, `--json=true`, `add`, `remove`, `clear`, path
operands, extra arguments, and non-Unicode arguments are invalid. The complete
grammar is validated before current-directory or any other native authority is
acquired.

Invalid syntax writes the one global usage diagnostic to standard error,
writes no standard output, and exits `2`. Success writes only standard output
and exits `0`. An operational or rendering failure exits `1`. Human mode writes
its fixed diagnostic to standard error with empty standard output. JSON mode
writes one compact error object to standard output with empty standard error.
A stdout failure uses the existing exact
`machine-god: failed to write output\n` standard-error diagnostic and exits
`1`.

The closed, redacted presentation categories are:

| Category | Meaning |
| --- | --- |
| `Unavailable` | Current-directory capture failed, or the captured path is non-Unicode, relative, or contains a lexical parent component. |
| `ResourceLimit` | The path, returned snapshot invariant, or complete serialized output exceeds its bound. |

The human failure is exactly
`machine-god workspace: could not inspect workspace: <Category>\n`. JSON
failure fixes key order `kind,error,code` and is exactly
`{"kind":"workspace","error":"could not inspect workspace: <Category>","code":"<Category>"}\n`.
Neither mode reflects a path, environment value, operating-system diagnostic,
raw error number, or underlying error text.

## Successful output

The result has one primary directory. It is the accepted lexical path returned
by the single process current-directory capture, not a canonical path, retained
descriptor, filesystem identity, or promise that the directory still exists.
The UTF-8 path is at most 4,096 bytes.

Human success is exactly these two LF-terminated lines:

```text
[workspace] primary="<absolute-path>"
[workspace] additional_directories=unsupported
```

`<absolute-path>` is a JSON string, including its quotes. Quotes, backslashes,
C0/C1 controls and DEL, Unicode line and paragraph separators, and Unicode
bidirectional-formatting controls are escaped. The path is intentional public
output in both modes.

Compact JSON fixes key order
`kind,action,primary_directory,additional_directories_supported,additional_directories`:

```json
{"kind":"workspace","action":"list","primary_directory":"<absolute-path>","additional_directories_supported":false,"additional_directories":[]}
```

`action` is always `list`. The `false` value and empty array describe the
currently supported command surface; they do not claim that an upstream or
foreign saved-directory configuration was inspected. Both representations are
assembled completely before their first byte is written, end with exactly one
LF, and are capped at 32,768 bytes including that LF. A violated result
invariant or output cap fails atomically as `ResourceLimit`; partial success
output is never intentionally emitted.

## Native authority and effects

After parsing, the native workspace inspection boundary synchronously calls
`std::env::current_dir()` exactly once. The returned path must be nonempty,
Unicode, absolute, contain no lexical `ParentDir` component, and satisfy the
4,096-byte UTF-8 limit. Inspection performs lexical validation only.

The command does not read process environment variables, configuration, state,
credentials, session records, or directory metadata. It does not inspect,
canonicalize, open, create, remove, rename, or write a filesystem object; load
or prepare native roots; construct an engine, provider, transport, runtime, or
reference host; prompt; or use the network. In particular, missing, empty,
relative, non-Unicode, or otherwise invalid `HOME`, `XDG_CONFIG_HOME`, and
`XDG_STATE_HOME` values cannot affect this command and no configuration or state
path is created.

This boundary is separate from `NativeRootSelection::from_current_process`.
Root selection also selects a state root and therefore has authority and
failure modes that a primary-workspace observation does not need. The workspace
snapshot grants no filesystem or tool authority to later operations.

## Deferred surface

The following pinned-upstream behavior remains intentionally unsupported:

- `workspace add PATH`, `workspace remove PATH`, `workspace clear`, durable
  additional-directory configuration, reconciliation, saved suppression, and
  the upstream additional-directory limit;
- global `--add-dir` and `--no-additional-dirs` options;
- availability, activation, and source flags for additional directories;
- interactive `/workspace` behavior and its slash-command category;
- extending tool authority, indexing, search, or completion across additional
  roots; and
- canonical filesystem identity, descriptor retention, or a compatibility-
  equivalence claim for the broader upstream workspace manager.
