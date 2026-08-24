# Native `create_folder` contract

Status: **CYCLE 4 PRODUCTION REVIEW GREEN; DOCUMENTATION-SEAL FINDING FIXED;
DELIVERY PENDING**

This document records the twenty-fifth bounded Milestone 03 slice from exact
delivered base `d1a5bc24112bcede8c2d12789e763a12cf44bd4a`. That base is green
under exact feature CI `32685885104`, feature benchmark workflow `32685885086`,
main CI `32686210561`, and main benchmark workflow `32686210659`. Both
benchmark workflows retain exactly two nonexpired exact-SHA artifacts.

The normative contract was frozen in exact documentation-only commit
`9fab189c9c1add76a38775d08f4342c6bcc7635b`. Its exact CI `32687614476`
passed all six jobs, and exact benchmark workflow `32687614442` passed both
jobs and retains exactly two nonexpired exact-SHA artifacts. Those workflows
validate the contract checkpoint only; they are not implementation, delivery,
performance, or fx-equivalence evidence.

Production behavior, exports, independently owned evidence, and both reference-
host constructors are composed. Tree-identical cycle-2 candidate
`6e1f885aa1e167e902b5cda729023fd7c283895e`, tree
`ac57575c3ee300050f5a92d4cae5f507fe654002`, is historically not green:
correctness/API and performance/concurrency are green with zero findings, while
filesystem/robustness reported two low evidence/documentation findings and zero
production defects. Exact remediation
`f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`, tree
`40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
local gate. Documentation record `9d0bacd656d09b8ff57edfbfe7cbf701af9fef1e`,
tree `b5fb1c2b5268e46793d48be1a02611381feca7c3`, and tree-identical cycle-3
candidate `c1e572eb1ac1ac39a8a53f522e74f57fd1d4f85d` retain identical non-
documentation behavior. Cycle 3 is not green: filesystem/robustness and
performance/concurrency are green with zero findings, while correctness/API
reported one low documentation-lineage finding and zero production defects.
Exact lineage remediation `12c11baa0187f530a6c088326b869991f6f627f6`, tree
`b96575b57c2c805a845294ae16b323dd1ea4ecd2`, passes the complete replacement
gate. Documentation gate record `f6f65847a47a009b5203044ce18e6f0c4253f17a`
is the parent of tree-identical cycle-4 candidate
`a78b693e5ce45688084fe1215073e2d859f2d438`, tree
`2b913e8d65b1da518f2c148f1b9b1b6b899e1e64`. Correctness/API and performance/
concurrency are green with zero findings. Filesystem/robustness found zero
production defects and one low stale documentation-seal sentence, corrected
under the user's explicit seal-review exemption. Feature delivery workflows,
`main` integration, and exact `main` workflows remain pending. No CLI behavior,
benchmark workload, product-performance claim, or fx-equivalence claim is
added.

`create_folder` creates one confined directory path, including its missing
parent directories. It does not create a file, overwrite or remove an entry,
follow a symlink, enumerate or read content, accept an external path, normalize
effective permissions after creation, or roll back a created prefix. It is
library-only in this slice. The product remains Rust; Zig remains solely a
pinned upstream benchmark build input.

## Public API and schema

The composed `machine-god-native` implementation exports
`CREATE_FOLDER_TOOL_NAME`, `CreateFolderTool`, `CreateFolderToolOpenError`,
`CreateFolderToolOpenErrorKind`, and these limits:

| Public constant | Exact value |
| --- | ---: |
| `CREATE_FOLDER_TOOL_NAME` | `"create_folder"` |
| `MAX_CREATE_FOLDER_PATH_BYTES` | `4,096` |
| `MAX_CREATE_FOLDER_PATH_COMPONENTS` | `256` |
| `MAX_CREATE_FOLDER_MKDIR_CALLS` | `256` |
| `MAX_CREATE_FOLDER_SYNC_CALLS` | `4,112` |
| `MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES` | `65,536` |
| `MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES` | `16,384` |

The exact tool description is
`Create one directory path and missing parents within the configured workspace`.
The exact `path` property description is
`Workspace-relative directory path`.

The exact input schema is:

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Workspace-relative directory path"
    }
  },
  "required": ["path"],
  "additionalProperties": false
}
```

`path` is a required string with no default. Unknown fields are invalid. The
requested and canonical path are each capped at 4,096 UTF-8 bytes, with at most
256 canonical components. Complete requested and prepared JSON values are each
capped at 65,536 serialized bytes. Direct execution revalidates the exact
shape, bounds, and canonical representation.

Normalization is the delivered mutation-path rule: repeated `/` separators
collapse and exact `.` components disappear. Backslash and space remain literal
Unix filename characters. Empty paths, absolute paths, `~`-prefixed paths, any
`..` component, C0/C1 controls, Unicode line or paragraph separators, Unicode
bidirectional-formatting characters, and a canonical `.` path are rejected.

Construction accepts one injected absolute workspace directory. The public API
and fixed unsupported result exist on every target; execution is supported only
on Linux and macOS. Construction errors retain only their kind:

| Kind | Exact `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native create_folder is unsupported on this platform` |
| `InvalidRoot` | `native create_folder workspace root is invalid` |
| `InvalidFileType` | `native create_folder workspace root is not a directory` |
| `Unavailable` | `native create_folder workspace root is unavailable` |

`Display` and `Debug` never retain the requested path, canonical path, injected
root, operating-system text, or raw error number.

## Preparation and authority

Preparation is deterministic, synchronous, bounded, nonblocking, and effect-
free. It performs no lookup, open, metadata read, creation, mutation, root
inspection, or permission change. Successful preparation retains exactly the
canonical path and returns:

```text
Capability::Filesystem {
    access: FilesystemAccess::Create,
    path: canonical_path,
}
```

Core already exposes `FilesystemAccess::Create`; no new core capability variant
is required. Its stable serialized permission input is exactly:

```json
{"type":"filesystem","access":"create","path":"canonical/path"}
```

Policy and allowed execution receive the same canonical path. Denial or failed
preparation has no filesystem effect. The capability grants only the bounded
creation, validation, and durability operations below. It grants no content
read, enumeration, file creation, overwrite, deletion, external-path access,
symlink following, chmod, ACL rewriting, or unrelated filesystem authority.

## Supported creation protocol

Allowed Linux/macOS execution uses only the retained workspace descriptor and
per-call owned descriptors. The injected pathname is never reopened as
authority. For one call it performs this bounded sequence:

1. Check cancellation, acquire `.` descriptor-relatively, and validate the
   exact linked workspace identity.
2. Walk the canonical components from the retained root using descriptor-
   relative, no-follow, nonblocking directory opens. Reject every symlink and
   non-directory ancestor. Record the existing prefix identities and retain
   its final directory descriptor.
3. If the final component already exists as a no-follow directory, perform the
   final precommit cancellation ordering and return idempotent success. If it
   exists as any other entry type, return the fixed target-exists failure.
4. Reacquire and validate the linked root, freshly rewalk the existing prefix,
   require its recorded identities, and make the final precreation
   cancellation check.
5. For each missing component, invoke `mkdirat` once with requested mode
   `0755`, then open and validate the resulting entry no-follow as a directory.
   A raced `EEXIST` is accepted only when that no-follow validation succeeds.
   A symlink or non-directory appearance fails closed. No `mkdirat` call is
   retried, including after `EINTR`, and at most 256 calls are made.
6. From the first successful or uncertain creation onward, retain bounded
   descriptors from the parent of that component through every subsequently
   created or idempotently accepted suffix directory, including a directory
   accepted after a raced `EEXIST`. After creation, freshly rewalk root-to-final
   no-follow and require the public path to identify the final retained
   directory chain.
7. Whether or not that postcommit public-path verification succeeds, always
   attempt bottom-up synchronization of every retained suffix directory and
   the parent of the first-created or uncertain directory. At most 257 sync
   sites are visited; each permits at most 16 cumulative `fsync` calls including
   interruptions, for the public total bound of 4,112 calls.

Success is exactly:

```json
{"path":"canonical/path"}
```

The same path-only result is used for new creation and an already-existing
final directory. There is no `created` flag or created-component count. The
complete `ToolOutput` is defensively capped at 16,384 serialized bytes.

## Permission and ACL boundary

Every `mkdirat` requests mode `0755`. Effective mode and ACLs deliberately
honor the host process umask, default ACLs, and platform inheritance. The tool
does not call `chmod`, `fchmod`, `fchmodat`, or rewrite, clear, or normalize an
ACL after creation. Therefore it promises neither exact `0755` effective mode
nor an empty ACL.

This differs intentionally from fixed native state-root suffix creation. These
names are caller-selected beneath potentially mutable directories, so a later
pathname-based normalization would create a replacement race and broader
mutation authority. A hostile umask can remove owner search or write bits from
a newly created intermediate directory. The tool may then be unable to reopen
or continue through its own new prefix and must return commit ambiguity after
bounded durability attempts; the partial prefix remains.

## Commit boundary, cancellation, and durability

The first successful `mkdirat`, or the first `mkdirat` result that cannot prove
no creation occurred, is the irreversible boundary. In particular, `EINTR` is
uncertain and is never retried. Before that boundary, saved native errors use
check-before/call/check-after cancellation ordering, and cancellation wins a
saved precommit error. If no creation call succeeds or is uncertain, failure or
cancellation has no tool-created filesystem effect and no sync is attempted.

At and after the commit boundary, cancellation is ignored through bounded
validation and durability work. The tool never calls `rmdir`, rolls back, or
removes a created prefix. A later validation, creation, permission, or sync
failure may leave some or all missing components present and returns fixed
nonretryable commit ambiguity. Every retained sync site is attempted bottom-up
even if an earlier site or the fresh public-path verification fails.

The execution future is inert until first poll and performs the complete
bounded synchronous operation in that poll. It starts no task, thread, process,
timer, or runtime. Cancellation cannot preempt a syscall already in flight.
Dropping the future before first poll has no effect; drop during or after the
synchronous poll starts no cleanup work, and owned descriptors close by RAII.

## Fixed tool errors

All failures are fixed and redacted:

| Code | Kind | Retryable | Exact message |
| --- | --- | --- | --- |
| `create_folder_invalid_arguments` | `InvalidInput` | no | `create_folder arguments are invalid` |
| `create_folder_invalid_path` | `InvalidInput` | no | `create_folder path is invalid` |
| `create_folder_unsupported_platform` | `Unavailable` | no | `native create_folder is unsupported on this platform` |
| `create_folder_permission_denied` | `PermissionDenied` | no | `requested folder cannot be created` |
| `create_folder_path_rejected` | `PermissionDenied` | no | `requested folder path is not confined` |
| `create_folder_target_exists` | `Execution` | no | `requested folder path already exists as a non-directory` |
| `create_folder_unavailable` | `Unavailable` | yes | `requested folder is unavailable` |
| `create_folder_target_changed` | `Execution` | yes | `requested folder path changed during creation` |
| `create_folder_create_failed` | `Execution` | yes | `requested folder could not be created` |
| `create_folder_commit_ambiguous` | `Execution` | no | `requested folder creation status is uncertain` |
| `create_folder_cancelled` | `Cancelled` | no | `create_folder execution was cancelled` |

`create_folder_target_exists` is reserved for an existing final entry that is
not a no-follow directory, including a final symlink or special entry. A
symlink or non-directory selected as an ancestor is confinement rejection.
Errors retain no path, root, component, entry name, mode, ACL, device/inode,
OS diagnostic, or errno. Engine-facing failures remain the delivered generic
durable tool-error surface.

## Races and confinement boundary

A concurrent directory appearance after an observed missing component may be
validated no-follow and used under idempotent semantics. A symlink or
non-directory appearance fails closed. Retained descriptors prevent pathname
replacement from redirecting already-open traversal, but a retained parent
moved elsewhere before a descriptor-relative creation can receive the effect.
Fresh root-to-final verification prevents that observation from being reported
as success; it cannot roll back the created entry, so the result is ambiguity.

The operation is not a filesystem transaction or sandbox. Another actor may
change names immediately after validation or success. Resolution of ancestor
components leading to the injected root and mounts visible beneath it remain
trusted host authority.

## Host composition and compatibility boundary

The last delivered reference host remains exactly the ten alphabetical tools:
`copy_file`, `delete_file`, `edit_file`, `file_info`, `glob_files`,
`grep_files`, `list_files`, `read_file`, `rename_file`, and `write_file`, using
the original retained descriptor plus nine identity-preserving clones.

The composed slice-twenty-five source registers `create_folder` immediately
after `copy_file`, yielding exactly eleven alphabetical tools and using one
original retained descriptor plus ten identity-preserving clones. Both path-
based and prepared-root constructors compose that same catalog and retained
workspace identity. Exact remediation `f527293`, tree `40eef14`, passes the
complete replacement local gate after cycle 2, which was not green. Cycle-3
candidate `c1e572e`, tree `b5fb1c2`, is not green only for one low documentation-
lineage finding. Exact lineage remediation `12c11ba`, tree `b96575b`, passes the
complete replacement gate. Cycle-4 candidate `a78b693`, tree `2b913e8`, has
zero production findings; its sole low stale seal-record finding is corrected
in the exempt documentation seal. It is not a delivered or `main`-integrated
authority surface.

Pinned fx at `b1774fbf6c7602b503026f96f6e960e946c692ef` uses the same tool
name and required `path` field, recursively creates missing parents, treats an
existing final directory as success, and rejects an existing final
non-directory. It also accepts absolute, `~/...`, and escaping relative paths,
uses broader pathname resolution that can follow symlinks, and lacks this
slice's strict shape, confinement, explicit bounds, durability protocol, and
redacted diagnostics. Machine-god intentionally rejects those broader
behaviors. Zig is benchmark input only.

## Required implementation evidence

- [x] Exact constants, descriptions, strict schema, construction taxonomy,
  success shape, error codes/messages/kinds/retryability, and redaction.
- [x] Exact and one-over requested/canonical path, component, serialized
  argument, serialized result, 256-`mkdirat`, 257-site, 16-call-per-site, and
  4,112-total sync bounds.
- [x] Effect-free preparation, exact stable `Create` capability JSON, denial
  before lookup, canonical direct execution, and policy/execution agreement.
- [x] Single and 256-component creation, recursive missing-parent creation,
  existing-directory idempotence, existing-final-nondirectory failure, and no
  file/content/enumeration/delete/overwrite authority.
- [x] Ancestor and final symlink, file, FIFO, socket, and device rejection;
  concurrent directory and hostile-entry appearance; root and prefix
  replacement; moved retained parents; deterministic mixed-device identity
  traversal; outside sentinels. This is not privileged real-mount testing or a
  sandbox guarantee.
- [x] Requested `0755`, benign and hostile umasks, inherited ACLs, no
  permission or ACL rewriting, unopenable new intermediate ambiguity, and
  retained safe partial prefixes.
- [x] Exact-once `mkdirat`, no retry after any result including `EINTR`, first-
  effect commit transition, fresh postcommit no-follow rewalk, bottom-up
  best-effort sync despite earlier verification/sync failure, and no rollback.
- [x] Precommit cancellation ordering and precedence, postcommit cancellation
  suppression, inert-until-poll, synchronous one-poll completion, drop, no
  detached work, and same-poll engine unknown-result recovery.
- [x] Native macOS execution, Linux and FreeBSD cross-target test compilation,
  Linux library warnings-denied Clippy, WASI compilation and active unsupported-
  target behavior, exact delivered ten-tool checkpoint, composed eleven-tool/
  ten-clone candidate, no-unsafe, dependency, compatibility, documentation,
  clean-diff, and fresh release-binary smoke evidence.
- [ ] Native Linux execution under exact feature CI.

The historical cycle-2 precursor local gate recorded in the
[`create_folder` review](reviews/m03-create-folder-review-01.md) passed 16
private, 20 direct, six engine, seven reference-host, and one core-contract
focused tests; 877 default and 925 all-feature discovered tests with zero
benchmarks; workspace formatting, warnings-denied Clippy, tests, and doctests;
130 Python tests with eight expected macOS-only skips; compatibility,
dependency, native macOS execution, Linux/FreeBSD cross-target compilation,
Linux library Clippy, active WASI, documentation, diff, and release-binary
checks. Cycle-2 evidence remediation added a seventeenth private deterministic
mixed-device identity-chain test.

Exact remediation `f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`, tree
`40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
local gate with fresh evidence:

- exact Rust 1.94.1 workspace formatting, all-target/all-feature warnings-
  denied Clippy, full workspace tests, and two workspace doctests;
- 17 private, 20 direct, six engine, seven reference-host, and one core-
  contract focused tests; 878 default and 926 all-target/all-feature discovered
  tests with zero benchmarks;
- 130 repo-wide Python tests with eight expected macOS skips;
- byte-identical compatibility regeneration against pinned fx revision
  `b1774fbf6c7602b503026f96f6e960e946c692ef`;
- `cargo-deny` 0.20.2 green and `cargo-audit` 0.22.2 green with zero findings
  across 175 dependencies and 1,225 advisories;
- Linux and FreeBSD cross-target checks and Linux warnings-denied library
  Clippy only, plus WASI compilation and active Node 22.22.0 evidence 1/1;
- 70 Markdown files, 502 links, 352 relative file links, and zero missing
  targets; clean diff, no unsafe additions, and no Cargo manifest, lockfile,
  CLI, workflow, or benchmark-workload changes; and
- a fresh 319,152-byte Mach-O arm64 release binary with SHA-256
  `71e7bfc79acc08fb3037b36f8b45ed24f9bbf9b9158dae359b5f544fa1e0fe78`
  passing bare, version, help, and inert missing-path human/JSON smokes.

Native Linux execution remains pending exact feature CI. Local-gate success is
not delivery, product-performance, or fx-equivalence evidence. Documentation
record `9d0bacd`, tree `b5fb1c2`, and tree-identical cycle-3 candidate `c1e572e`
preserve identical non-documentation behavior. Cycle 3 is not green only for
one low documentation-lineage finding; the other two tracks are green and all
three found zero production defects. Exact lineage remediation `12c11ba`, tree
`b96575b`, passes the complete replacement gate. Gate record `f6f6584` parents
tree-identical cycle-4 candidate `a78b693`, tree `2b913e8`. Cycle 4 found zero
production defects; its sole low stale documentation-seal finding is corrected
under the user's seal-review exemption. Production preflight API and filesystem
audits reported zero findings, but they are not formal tracks.

## Review and delivery protocol

Production and independently owned evidence are composed. Run the complete
local gate on one exact SHA, then create a tree-identical candidate and start
three fresh reviewers against that same immutable SHA and tree:

1. correctness/API;
2. filesystem/robustness;
3. performance/concurrency.

Every confirmed finding is fixed, the complete local gate is rerun, and all
three tracks restart with fresh reviewers on one replacement SHA. Repeat until
all three tracks report zero findings. Then push the feature seal, require its
exact CI and benchmark workflows, fast-forward `main` without force, and
require exact `main` CI and benchmark workflows. Documentation-only seal and
delivery-record commits are exempt from another adversarial cycle, but their
exact workflows remain required.

## Deferred scope

File creation, overwrite, removal, rollback, exact effective mode, ACL
normalization, external paths, symlink traversal, directory enumeration,
content access, non-Linux/macOS hardened execution, CLI ownership, benchmark
workloads, product-performance claims, and complete fx equivalence remain
outside this slice. The replacement complete local gate is green at
`f527293`, tree `40eef14`. Cycle-3 candidate `c1e572e`, tree `b5fb1c2`, is not
green only for one low documentation-lineage finding. Exact lineage remediation
`12c11ba`, tree `b96575b`, passes the complete replacement gate. Cycle-4
candidate `a78b693`, tree `2b913e8`, has zero production findings; its sole low
stale seal-record finding is corrected in this exempt documentation seal.
Delivery and `main` integration remain pending; the delivered-slice count stays
twenty-four.
