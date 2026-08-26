# Top-level sessions command

Status: delivered as bounded Milestone 03 slice 31. Exact candidate `a527652`,
tree `0249dd0`, passed three fresh cycle-2 review tracks at `0/0/0/0` each.
Review-exempt seal `b5b9116`, tree `3e61754`, passed exact feature and integrated
`main` CI/benchmark workflows with two unexpired exact-SHA artifacts in each
benchmark run. Full evidence is retained in the
[review ledger](reviews/m03-sessions-cli-review-01.md).

The command exposes the existing bounded native session-ID observation through
the thin CLI. It does not construct an engine, provider, model transport,
permission handler, workspace tool, network runtime, or full native reference
host.

## Grammar and exits

The only accepted invocations are:

```text
machine-god sessions
machine-god sessions --json
```

The singleton `--json` flag must be second. `--all`, `--limit`, `--cursor`,
`--json=true`, repeated flags, extra positional arguments, reordered flags, and
non-Unicode arguments are invalid. Parsing completes before environment or
filesystem access. Invalid syntax writes the one global usage diagnostic to
standard error, writes no standard output, and exits 2.

A successful complete or truncated observation exits 0 with empty standard
error. An operational or rendering failure exits 1. Human mode writes its
fixed diagnostic to standard error with empty standard output. JSON mode writes
one compact error object to standard output with empty standard error. Output
failure uses the existing fixed `machine-god: failed to write output` standard-
error diagnostic and exits 1.

Operational categories are closed and redacted:

| Category | Meaning |
| --- | --- |
| `Corrupt` | A selected canonical record failed the native record contract. |
| `Unavailable` | State selection, root safety, or bounded persistence work failed. |
| `Unsupported` | The current target has no supported native listing implementation. |
| `ResourceLimit` | A host/result invariant or serialized-output ceiling failed. |

The human failure is exactly
`machine-god sessions: could not list sessions: <Category>\n`. The JSON failure
fixes key order `kind,error,code` and is exactly
`{"kind":"sessions","error":"could not list sessions: <Category>","code":"<Category>"}\n`.
Neither form reflects paths, environment values, record data, filenames,
operating-system diagnostics, or raw error numbers.

## Successful output

The result contains no more than 100 validated session IDs in strict ascending
lexical order with no duplicates. Each ID uses the core portable identifier
alphabet and is at most 128 bytes. `truncated` means only that a scan, aggregate-
byte, or result bound prevented an exhaustive observation. It is successful
data, not an error, a `has_more` promise, or a pagination token.

An empty complete human result is exactly:

```text
[sessions] no saved sessions
```

A nonempty result starts with `[sessions] N saved`, followed by one
` - <id>` line per ID. An empty truncated result uses that counted header with
`N` equal to zero rather than the complete-empty sentence. Any truncated result
ends with:

```text
[sessions] listing incomplete: a resource limit was reached
```

JSON fixes top-level key order `kind,count,truncated,sessions`. `sessions` is an
array in the same order as the human rows; every element currently has the sole
key `id`:

```json
{"kind":"sessions","count":2,"truncated":false,"sessions":[{"id":"alpha"},{"id":"beta"}]}
```

`count` equals the array length. Both modes have exactly one final LF. The
complete representation is built before the first success byte is written and
is capped at 16 KiB including that LF. This ceiling covers 100 maximum-length
valid IDs in either format. A violated result invariant or output cap fails as
`ResourceLimit`; partial success output is never intentionally emitted.

## Native state and effects

On Linux and macOS the first poll selects nonempty `XDG_STATE_HOME`, otherwise
nonempty `HOME` plus `.local/state`, and then the fixed `machine-god` namespace.
The process facade requests `XDG_STATE_HOME` first and requests `HOME` only when
that value is missing or empty; it never requests `XDG_CONFIG_HOME`. A selected
invalid or non-Unicode nonempty XDG value fails without requesting or falling
back to `HOME`. No configured state environment fails redacted as `Unavailable`.

The native facade walks an existing selected state root descriptor-relatively
without following fixed suffix symlinks. It applies the same effective-user,
group/other-write, private-final-mode, and macOS ACL policy as native root
preparation, but it neither opens a workspace nor creates or repairs a missing
directory. A genuinely absent selected base or fixed suffix is an empty,
non-truncated success. Existing unsafe, inaccessible, symlink, or wrong-kind
components fail closed. Unsupported targets return `Unsupported` without
filesystem access.

The actual observation delegates to the delivered
[`NativeSessionList`](native-session-listing.md) scan. Its bounds remain 100
returned IDs, 1,024 processed non-dot entries plus one name-inspected overflow
witness, 64 MiB of accepted aggregate record bytes plus one transient byte
witness, and the existing per-record ceiling. A corrupt selected candidate
fails the whole call; concurrent disappearance may omit that candidate.

Constructing either injected-environment or process-environment future is
effect-free. The process snapshot, state selection, descriptor operations,
record reads, and advisory locks occur on first poll and complete synchronously
on that polling thread. There is no task, thread, timer, provider request,
permission prompt, network request, runtime construction, configuration read,
credential discovery, workspace access, or `.fx` access.

Listing creates no state root or record and never repairs, rewrites, deletes,
or migrates a record. As already documented by the native library, validating
an existing canonical record may create its missing permanent private `0600`
advisory-lock sidecar. This bounded side effect is explicit; the command is not
described as strictly no-write.

## Deliberate pinned-fx differences

Pinned fx supports `--all`, `--limit`, and `--cursor`, defaults to the current
workspace, returns rich summary objects, orders newest first, supplies cursors,
and skips/report corrupt records. Current machine-god records have no
authoritative workspace, title, preview, language, creation time, update time,
history length, or display rank. This slice therefore lists the selected
machine-god namespace globally, returns ID-only objects in lexical order,
exposes bounded truncation without pagination, and fails the call on selected
corruption.

The pinned `sessions-json` benchmark record moves only from `unimplemented` to
implemented-but-`non-equivalent`. Both commands remain `not-measured` and
`claim_eligible: false`; no samples, thresholds, product-performance result,
compatibility promotion, or fx-equivalence claim are introduced. Zig remains
only the pinned toolchain used to build the upstream comparison input;
machine-god remains a Rust product.
