# Native `delete_file` contract

Status: **DELIVERED — final documentation record workflows pending**

This document freezes the twenty-second bounded Milestone 03 slice from exact
delivered base `719a9bded86fd7ce394d482798b9064c736f43ab`. That base is green
under exact feature CI `32651168514` across all six jobs and feature benchmark
workflow `32651168515` across both jobs with two nonexpired exact-SHA artifacts.
`main` was fast-forwarded without force from
`c1268fdf463e11242b7b916add70675ae91ed115` to that exact base and is green
under exact CI `32651488265` across all six jobs and benchmark workflow
`32651488282` across both jobs with two nonexpired exact-SHA artifacts.

Documentation-only contract commit
`78ed6292386f86e5807bcf72591d6cb5d9f45c45` is green under exact feature CI
`32652361712` across all six jobs and benchmark workflow `32652361692` across
both jobs with two nonexpired exact-SHA artifacts. Those workflows froze only
the contract. Production and independently owned evidence are now composed
through exact local-gate precursor
`5e340155f9a38b81a2812942d6ad0a796164beb5`. Formal cycle 1 reviewed exact
behavior candidate `7c6f7eed407f93d2ae335e6e3b5b4ad099a615cf` and all three
tracks reported **NOT GREEN**. Remediation, replacement same-SHA review, and
remote delivery remain pending. Exact remediation
`60e81a633557bc90aca01e3579782340c7c154c9` passes the complete replacement
local gate. Tree-identical formal cycle-2 candidate
`88026f10ed8c194c7160a754f226241c276579fc` is **NOT GREEN**;
exact remediation `225e9617a8a8f469d663693b61cc4f9b97af8094` passes the
complete replacement local gate. Tree-identical formal cycle-3 candidate
`24f851d2d3db21735124729bb1b0a14adf7ae864` is **NOT GREEN** with
two low findings. Exact remediation
`77884a9fceed6268cbdbec1310de3f94a9c5a230` passes the complete replacement
local gate. Tree-identical formal cycle-4 candidate
`0b732d2746d5c821a5294901f8b4cc641bc98530` is **NOT GREEN** with
one overlapping medium finding. Exact remediation
`4273de513007175be94829aef85aaaa0d09bc02c` passes the complete replacement
local gate. Tree-identical formal cycle-5 candidate
`8575354542803f5e8ba8faf311e7524ed87eacba` is **GREEN** with zero findings
in all three fresh tracks. Documentation seal
`9e2a2764420519a94e11986a758592b442faa65d` passed exact feature benchmark
workflow `32663557187` across both jobs with two nonexpired exact-SHA artifacts,
but exact feature CI `32663557182` failed one of six jobs because its aarch64
Linux filesystem immediately reused a test fixture's unlinked inode. Exact
test-only remediation `c6744ab5416fc4bde330d09f59dd507bd9991d72`
passes the complete replacement local gate. Tree-identical cycle-6 candidate
`9e817beb92b14ce718c9c6a2b35637fb6fa2cf7e` is **GREEN** with zero
findings in all three fresh tracks. Replacement documentation seal
`fe56f4c57ef18f87c742340a6060dc56b91f00f9` is green under exact feature CI
`32665295323` across all six jobs and benchmark workflow `32665295321` across
both jobs with two nonexpired exact-SHA artifacts. `main` was fast-forwarded
without force from the delivered base to that seal and is green under exact CI
`32665564381` across all six jobs and benchmark workflow `32665564382` across
both jobs with two nonexpired exact-SHA artifacts. Native `delete_file` is
delivered as the twenty-second bounded Milestone 03 slice.

`delete_file` deletes exactly one existing confined regular file or empty
directory. It does not recurse, follow a symlink, remove the workspace root,
read file content, enumerate a directory, create a path, or grant general
filesystem authority. It is library-only in this slice. The product remains
Rust; Zig remains solely a pinned upstream benchmark build input.

## Public API, schema, and limits

`machine-god-native` exports `DELETE_FILE_TOOL_NAME`, `DeleteFileTool`,
`DeleteFileToolOpenError`, `DeleteFileToolOpenErrorKind`, and these public
limits:

| Public constant | Exact value |
| --- | ---: |
| `DELETE_FILE_TOOL_NAME` | `"delete_file"` |
| `MAX_DELETE_FILE_PATH_BYTES` | `4,096` |
| `MAX_DELETE_FILE_PATH_COMPONENTS` | `256` |
| `MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES` | `65,536` |
| `MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES` | `16,384` |

The exact tool description is
`Delete one regular file or empty directory within the configured workspace`.
The exact `path` property description is
`Workspace-relative file or empty-directory path`.

The advertised schema and preparation input are exactly:

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Workspace-relative file or empty-directory path"
    }
  },
  "required": ["path"],
  "additionalProperties": false
}
```

`path` must be a string. It has no default, and no unknown field is accepted.
The requested path's lexical validity and 4,096-byte bound are checked before
the serialized argument object is independently capped at 65,536 bytes. The
remaining canonical path and component bounds follow normalization before
canonical arguments are retained. Direct execution revalidates the same exact
shape and precedence, so bypassing provider preparation cannot widen authority
or impose unbounded serialized-value work through an over-limit path string.

`DeleteFileTool::open` accepts one explicitly injected absolute workspace root.
Its complete fixed construction taxonomy is:

| `DeleteFileToolOpenErrorKind` | Exact `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native delete_file is unsupported on this platform` |
| `InvalidRoot` | `native delete_file workspace root is invalid` |
| `InvalidFileType` | `native delete_file workspace root is not a directory` |
| `Unavailable` | `native delete_file workspace root is unavailable` |

Construction errors retain only their kind. Their `Display` and `Debug` forms
must not retain the injected root, operating-system text, or a raw error number.
The public API and fixed unsupported behavior exist on every target, while
native execution is supported only on Linux and macOS.

## Preparation and authority

Preparation is deterministic, synchronous, bounded, nonblocking, and effect-
free. It performs strict JSON validation and the exact lexical mutation-path
normalization delivered for `write_file` and `edit_file`; it does not inspect
the retained root or requested target.

Repeated `/` separators collapse and exact `.` components are removed.
Backslash and space remain literal Unix filename characters. Empty paths,
absolute paths, any `..` component, C0/C1 controls, Unicode line or paragraph
separators, Unicode bidirectional-formatting characters, and a canonical `.`
target are rejected. Rejecting `.` makes workspace-root removal impossible.
Both the requested UTF-8 spelling and canonical path are independently capped
at 4,096 bytes, and the canonical path is capped at 256 components.

Successful preparation retains exactly `{"path":"<normalized>"}` and returns:

```text
Capability::Filesystem {
    access: FilesystemAccess::Delete,
    path: normalized_path,
}
```

`FilesystemAccess::Delete` is the existing provider-neutral deletion authority.
Policy and allowed execution receive the exact same canonical path. A denied or
failed preparation performs no filesystem lookup, deletion, or sync.

## Supported deletion protocol

Allowed Linux/macOS execution uses only the workspace descriptor retained at
construction. Root construction uses the delivered lexical host-root handling,
final-component no-follow open, and exact linked-root validation. The host path
string is not reopened as authority after construction.

For one allowed call, execution performs this bounded sequence:

1. Check cancellation, acquire `.` descriptor-relatively from the retained
   workspace descriptor, and validate the exact platform-specific linked-root
   identity.
2. Walk every existing parent component through retained descriptor-relative
   directory opens with no-follow, nonblocking, and close-on-exec requirements.
   Record the final parent's device, inode, and directory type.
3. Inspect the target basename once with no-follow descriptor-relative
   metadata. Record its device, inode, and exact regular-file or directory type.
   A symlink or any other special type is rejected. The target is not opened,
   its content is not read, and a directory is not enumerated.
4. Reacquire and revalidate the linked root, completely rewalk the existing
   parent path, and require the final parent identity and directory type to
   equal the initial observation. Reinspect the target with no-follow metadata
   and require the same device, inode, and exact type.
5. Perform the final cancellation check immediately after final validation and
   immediately before one deletion syscall. Invoke exactly one `unlinkat`: a
   regular file uses empty flags and a directory uses the platform `REMOVEDIR`
   flag. Never retry that `unlinkat`, including after `EINTR`.
6. Once `unlinkat` reports success, ignore later tool cancellation and `fsync`
   the retained final-parent descriptor. The complete parent-sync phase accepts
   at most 16 cumulative interrupted results. The sixteenth interruption or
   any other sync failure returns nonretryable commit ambiguity.

An `unlinkat` `EINTR` is itself nonretryable commit ambiguity because the
directory entry may already have been removed. The tool makes no second delete
attempt. It best-effort syncs the retained parent under the same 16-interruption
bound before returning ambiguity, because a deletion that may have committed
still needs a durability attempt. Later cancellation does not replace that
ambiguous result.

A directory is never enumerated to predict emptiness. `unlinkat` with
`REMOVEDIR` is the authoritative empty-directory operation. A nonempty result
maps to the fixed nonretryable directory-not-empty error and leaves the entry
present. Regular-file deletion removes exactly the selected directory entry;
it does not destroy other hard links or invalidate already open descriptors.

Success is exactly:

```json
{"path":"normalized/workspace/path"}
```

No kind, byte count, inode, or deletion count is returned. The result is
defensively checked against the 16,384-byte serialized-result cap even though
the bounded canonical path makes a valid success structurally smaller.

## Commit boundary, cancellation, and races

The sole `unlinkat` call is the irreversible boundary. Before it is invoked,
every tool failure or cancellation publishes no deletion. A successful call
may have removed the entry even when later parent sync fails, so sync failure
is never retryable. Likewise, `EINTR` does not prove the syscall had no effect.
Callers must recover by observation instead of blindly retrying a commit-
ambiguous deletion.

The execution future is inert until polled, performs bounded synchronous work
on its polling thread, and starts no detached task, thread, subprocess, timer,
or runtime. Cancellation is checked before and after root acquisition, around
each parent open and metadata operation, after initial and final validation,
and at the exact final pre-`unlinkat` boundary. Every root/parent open and
metadata after-check runs even when that operation returns an error;
cancellation then takes precedence over the saved noncommit error. It cannot
preempt a syscall already in flight. Once the delete call has succeeded or
returned `EINTR`, later tool cancellation is ignored while the bounded parent
durability attempt completes. Dropping an unpolled future is effect-free;
dropping at a documented precommit boundary closes owned descriptors without
deleting anything.

When `unlinkat` returns a definitive non-`EINTR` failure, the target was not
committed by that outcome. Cancellation is therefore checked immediately after
the syscall/evidence hook and before errno or macOS diagnostic mapping; observed
cancellation wins as `delete_file_cancelled`. The failed call is not retried or
synced. Only success and `EINTR` cross the cancellation-ignore boundary.

This protocol is not pathname compare-and-swap. A same-directory actor can
replace the target after final no-follow validation and before `unlinkat`.
Because portable `unlinkat` accepts only a parent descriptor and name, a
regular-file deletion using empty flags may remove any non-directory entry
installed in that final window, including a different regular file, symlink,
FIFO, or Unix-domain socket. It never follows a replacement symlink, and the
symlink referent and unrelated sentinels remain untouched. A directory
replacement presented to the file-class call fails the flags/type boundary;
the inverse file-class replacement presented to a `REMOVEDIR` call likewise
fails. The directory-class call may still remove a different empty directory.
These portable final-window limits are disclosed and tested; the slice makes
no stronger adversarial concurrent-mutation or final-entry-type claim.

A retained final parent can be renamed outside the public workspace path after
validation. Descriptor-relative `unlinkat` can then remove the entry in that
same retained moved directory. Replacing the old public parent pathname does
not redirect the retained descriptor. Other hard links and already open file or
directory descriptors survive deletion as defined by the operating system. A
concurrent actor may also recreate the pathname immediately after successful
deletion, so success does not promise that a later lookup is absent.

There is no staged file, temporary name, content buffer, ACL operation, chmod,
rename, or cleanup-unlink protocol in this slice. Those `write_file` and
`edit_file` mechanisms are intentionally inapplicable to a one-entry delete.

## Fixed redacted errors

Preparation and direct execution use this complete fixed taxonomy. `Display`
is exactly `<code>: <message>`.

| Kind | Code | Message | Retryable |
| --- | --- | --- | --- |
| invalid input | `delete_file_invalid_arguments` | `delete_file arguments are invalid` | no |
| invalid input | `delete_file_invalid_path` | `delete_file path is invalid` | no |
| unavailable | `delete_file_unsupported_platform` | `native delete_file is unsupported on this platform` | no |
| unavailable | `delete_file_not_found` | `requested path is unavailable` | no |
| permission denied | `delete_file_permission_denied` | `requested path cannot be deleted` | no |
| permission denied | `delete_file_path_rejected` | `requested path is not a confined regular file or empty directory` | no |
| execution | `delete_file_directory_not_empty` | `requested directory is not empty` | no |
| unavailable | `delete_file_unavailable` | `requested path is unavailable` | yes |
| execution | `delete_file_target_changed` | `requested path changed before deletion` | yes |
| execution | `delete_file_delete_failed` | `requested path could not be deleted` | yes |
| execution | `delete_file_commit_ambiguous` | `requested path deletion status is uncertain` | no |
| cancelled | `delete_file_cancelled` | `delete_file execution was cancelled` | no |

An initially absent target or ancestor is `delete_file_not_found`. Absence or a
type mismatch during final revalidation or the delete call is
`delete_file_target_changed`. A nonempty directory is
`delete_file_directory_not_empty`. Permission and read-only-filesystem failures
are `delete_file_permission_denied` at every root, parent, and target operation
in either validation phase. `unlinkat` interruption and any failure after
successful deletion are `delete_file_commit_ambiguous`; `unlinkat` is never
retried. Other bounded operational failures map to the fixed unavailable,
target-changed, or delete-failed category according to the documented phase.

On macOS, empty-flag `unlinkat` reports `EPERM` for both a genuine permission
failure and a final-window file-to-directory replacement. The bounded
diagnostic no-follow metadata operation compares the complete observed target
identity and type with the validated regular-file identity. Cancellation wins;
absence, a type-change errno, or any observed identity/type mismatch is
`delete_file_target_changed`; an exact unchanged identity, diagnostic
`EACCES`/`EPERM`, or another diagnostic OS error preserves the original
`delete_file_permission_denied`. The diagnostic never retries deletion.

No public error or tool `Debug` form reflects the requested/canonical path,
workspace root, device or inode, file type, raw errno, operating-system text,
credentials, or another directory entry. Non-cancellation errors passing
through the engine retain core's existing generic durable error behavior.

## Reference-host composition

Current reference-host composition and retained-workspace descriptor
distribution are maintained in the
[canonical tool catalog](native-reference-host.md#tool-catalog).

The CLI remains byte-unchanged and thin. This slice adds no CLI command,
invocation path, prompt, status field, or output byte.

## Composed local evidence

Exact precursor `5e340155f9a38b81a2812942d6ad0a796164beb5` is locally green
under Rust and Cargo 1.94.1. Focused evidence passes 19 default-feature and 20
all-feature private tests, 19 direct tests, five engine tests, and seven
reference-host tests. Workspace formatting, all-target/all-feature warnings-
denied Clippy, workspace tests, and two doctests pass. Discovery inventories
728 default-feature tests, 778 all-feature tests, and zero benchmarks.

The repository harness passes all 130 tests with eight expected macOS skips.
Pinned-fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef`
compatibility, cargo-deny 0.20.2, and cargo-audit 0.22.2 over 1,225 advisories
and 175 dependencies are green. Linux cross-Clippy, FreeBSD library check and
Clippy plus unsupported-test type-check, and WASI library/test build gates pass;
Node actively executes the WASI unsupported-target test 1/1. The optional
all-feature Linux cross-build remains a host C-sysroot limitation in
`aws-lc-sys`, before product Rust compilation, rather than a `delete_file`
failure.

Documentation integrity covers 64 Markdown files, 445 inline links, and 295
repository-relative links with zero missing targets. The base diff is clean,
adds zero unsafe Rust, changes no Cargo metadata or CLI source, and changes only
Rust files under crates. A fresh locked arm64 Mach-O release CLI has SHA-256
`d5e91bac9cf07f389b98341ed0532d54d666f8aff2b92ffbd01f4a65cdfd8751`
and passes bare, help, and status smoke paths.

These are local precursor results only. Formal cycle 1 then reviewed exact
candidate `7c6f7eed407f93d2ae335e6e3b5b4ad099a615cf` and found four
unique issues: over-limit path work precedence, retained-root permission
taxonomy, macOS cancellation around post-`EPERM` diagnostic metadata, and an
under-specified non-directory replacement race. The cycle is **NOT GREEN** and
establishes no feature-delivery, `main`, compatibility, equivalence, or
product-performance approval.

Exact remediation `60e81a633557bc90aca01e3579782340c7c154c9` is green under
the complete replacement local gate on Rust and Cargo 1.94.1. Focused totals
are 22 default-feature and 23 all-feature private tests, 20 direct tests, five
engine tests, and seven reference-host tests. Workspace formatting, all-target/
all-feature warnings-denied Clippy, workspace tests, and two doctests pass;
discovery inventories 732 default-feature tests, 782 all-feature tests, and
zero benchmarks.

The 130-test Python harness passes with eight expected macOS skips. Pinned-fx
compatibility, cargo-deny 0.20.2, and cargo-audit 0.22.2 over 1,225 advisories
and 175 dependencies are green. Linux no-default-feature cross-Clippy,
FreeBSD library check and Clippy plus unsupported-test type-check, WASI
library/test builds, and Node's active unsupported test 1/1 pass. The optional
all-feature Linux cross-build remains blocked only by the host's missing Linux
C sysroot in `aws-lc-sys`, before product Rust compilation.

Documentation integrity remains 64 Markdown files, 445 inline links, 295
repository-relative links, and zero missing targets. The base diff is clean,
adds zero unsafe Rust, and changes neither Cargo metadata nor CLI source. A
fresh locked arm64 Mach-O release CLI is 319,152 bytes with SHA-256
`d143cb7ef8ba0871a4449cd1f3a6ebb868dcb0f43f433819ea5110698e260304`
and passes bare, help, and unavailable-environment status smoke paths with empty
stderr. These results qualify only the replacement local gate, not formal
review or delivery.

Formal cycle 2 reviewed exact tree-identical candidate
`88026f10ed8c194c7160a754f226241c276579fc`. Performance/concurrency is
**GREEN** with zero findings. Correctness/API and filesystem/robustness are
**NOT GREEN** with three overlapping medium findings: failed operations skipped
their after-cancellation checks, macOS `EPERM` diagnosis did not compare full
identity, and non-root revalidation permission errors lost the fixed taxonomy.
Correctness/API also found one low public-Rustdoc race-boundary mismatch.
Remediation and another complete local and fresh three-track cycle are pending.

Exact cycle-2 remediation `225e9617a8a8f469d663693b61cc4f9b97af8094`
passes the complete replacement local gate on Rust/Cargo 1.94.1. Focused totals
are 28 default-feature and 29 all-feature private tests, 20 direct tests, five
engine tests, seven reference-host tests, and one core `Delete` contract test.
Workspace formatting, all-target/all-feature warnings-denied Clippy, tests, and
two doctests pass; discovery inventories 738 default-feature tests, 788 all-
feature tests, and zero benchmarks.

The 130-test Python harness passes with eight expected macOS skips. Pinned-fx
compatibility, cargo-deny 0.20.2, cargo-audit 0.22.2 over 1,225 advisories and
175 dependencies, Linux/FreeBSD/WASI gates, and Node's active unsupported test
1/1 are green. Documentation integrity remains 64/445/295/0. The delivered-
base diff covers 16 files with 6,172 insertions and 63 deletions, adds zero
unsafe Rust, and changes no Cargo metadata or CLI source. The optional all-
feature Linux cross-build remains blocked in `aws-lc-sys` by the host's missing
Linux C sysroot before product Rust. A fresh locked 319,152-byte arm64 Mach-O
CLI has SHA-256
`951ff7ce945a6fa446dfd87a7d54a6dd962776a8a021d4af6e68d6bd18e963e8`
and passes bare, help, and unavailable-status smoke paths with empty stderr.
These results qualify the replacement local gate only, not formal review or
delivery.

Formal cycle 3 reviewed exact tree-identical candidate
`24f851d2d3db21735124729bb1b0a14adf7ae864`. Performance/concurrency is
**GREEN** with zero findings. Correctness/API is **NOT GREEN** with one low
finding because validation-site `EROFS` did not follow the frozen read-only-
filesystem permission taxonomy. Filesystem/robustness is **NOT GREEN** with one
low evidence gap: hostile-umask deletion was required by this matrix but not
retained in a deterministic regression. It found no production protocol defect,
and a manual restrictive-umask execution passed. Exact mapping/matrix and
isolated child-process evidence remediations are in progress; another complete
local gate and three fresh reviewers remain mandatory.

Exact cycle-3 remediation `77884a9fceed6268cbdbec1310de3f94a9c5a230`
passes the complete replacement local gate on Rust/Cargo 1.94.1. Focused totals
are 28 default-feature and 29 all-feature private tests, 21 direct tests, five
engine tests, seven reference-host tests, and one core `Delete` contract test.
Workspace formatting, all-target/all-feature warnings-denied Clippy, tests, and
two doctests pass; discovery inventories 739 default-feature tests, 789 all-
feature tests, and zero benchmarks.

The 130-test Python harness passes with eight expected macOS skips. Pinned-fx
compatibility, cargo-deny 0.20.2, cargo-audit 0.22.2 over 1,225 advisories and
175 dependencies, Linux/FreeBSD/WASI gates, and Node's active unsupported test
1/1 are green. Documentation integrity remains 64/445/295/0. The delivered-
base diff covers 16 files with 6,404 insertions and 63 deletions, adds zero
unsafe Rust, and changes no Cargo metadata or CLI source. The optional all-
feature Linux cross-build remains blocked in `aws-lc-sys` by the host's missing
Linux C sysroot before product Rust. A fresh locked 319,152-byte arm64 Mach-O
CLI has SHA-256
`951ff7ce945a6fa446dfd87a7d54a6dd962776a8a021d4af6e68d6bd18e963e8`
and passes bare, help, human-status, and JSON-status smoke paths with empty
stderr. These results qualify the replacement local gate only, not formal
review or delivery.

Formal cycle 4 reviewed exact tree-identical candidate
`0b732d2746d5c821a5294901f8b4cc641bc98530`. Correctness/API,
filesystem/robustness, and performance/concurrency all reported **NOT GREEN**
with the same single medium finding and no others: definitive non-`EINTR`
`unlinkat` errors mapped directly without observing cancellation raised during
the syscall. Source and representative file/directory errno-matrix evidence are
being remediated; another complete local gate and three fresh reviewers remain
mandatory.

Exact cycle-4 remediation `4273de513007175be94829aef85aaaa0d09bc02c`
passes the complete replacement local gate on Rust/Cargo 1.94.1. Definitive
non-`EINTR` deletion failures now check cancellation after the actual syscall
and evidence hook but before errno or macOS diagnostic mapping. A ten-case
file/directory failure matrix covers file `EIO`, `EACCES`, `EPERM`, `EROFS`,
`ENOENT`, `ENOTDIR`, `EISDIR`, and `ELOOP`, plus directory `ENOTEMPTY` and
`EEXIST`; every cancelled case retains target and sentinel state, performs one
delete call with exact flags, and performs zero syncs. Existing success and
`EINTR` regressions continue to prove later cancellation is ignored only after
the commit or ambiguity boundary.

Focused totals are 29 default-feature and 30 all-feature private tests, 21
direct tests plus the hostile-umask child, five engine tests, seven reference-
host tests, and one core `Delete` contract test. Workspace formatting, all-
target/all-feature warnings-denied Clippy, tests, and two doctests pass;
discovery inventories 740 default-feature tests and 790 all-feature tests,
including two doctests, with zero benchmarks.

The 130-test Python harness passes with eight expected macOS skips. Pinned-fx
compatibility, cargo-deny 0.20.2, and cargo-audit 0.22.2 over 1,225 advisories
and 175 dependencies are green. Linux no-default-feature cross-Clippy,
FreeBSD library check and Clippy plus unsupported-test type-check, WASI
library/test builds, and Node's active unsupported test 1/1 pass.
Documentation integrity remains 64/445/295/0. The clean delivered-base diff
covers 16 files with 6,582 insertions and 63 deletions, adds zero unsafe Rust,
and changes no Cargo metadata or CLI source. The optional all-feature Linux
cross-build remains blocked only by the host's missing Linux C sysroot in
`aws-lc-sys`, before product Rust compilation. A fresh locked 319,152-byte
arm64 Mach-O release CLI has SHA-256
`126ecc47857cb327e3b483daecf9c50ce6b04585f4cdaed60e6f20cb9f82b107`
and passes bare, help, human-status, and JSON-status smoke paths with exact
stdout and empty stderr. These results qualified only the replacement local
gate and required three fresh reviewers to green-light one tree-identical
behavior candidate before remote delivery.

Formal cycle 5 reviewed exact tree-identical candidate
`8575354542803f5e8ba8faf311e7524ed87eacba`, tree
`13f28f2a687960e17cd4061c849a0bae17604ae7`. Correctness/API,
filesystem/robustness, and performance/concurrency are all **GREEN** with zero
findings. Every reviewer used a clean detached worktree, verified the exact
candidate and tree, and confirmed that crate content is identical to exact
remediation `4273de513007175be94829aef85aaaa0d09bc02c`.

Correctness/API reran formatting, 29/30 private tests, 21 direct tests, five
engine tests, one core contract test, and all-feature reference-host
composition. Filesystem/robustness reran 29/30 private, 21 direct including the
hostile-umask child, five engine, and seven host tests plus FreeBSD/WASI checks
and the WASI unsupported-test build. Performance/concurrency reran 29/30
private, 21 direct including the hostile-umask child, and five engine tests.
All three independently confirmed the ten-case definitive-failure cancellation
matrix, the success/`EINTR` commit boundary, and closure of every historical
finding in their tracks.

This establishes behavior-green review only. Delivery still requires the
documentation seal's exact feature CI and benchmark workflows, a no-force
fast-forward to `main`, and exact `main` CI and benchmark workflows. The
documentation-only seal and later delivery record are exempt from another
adversarial cycle under the user's instruction, but not from their applicable
remote workflows.

The first documentation seal's exact feature CI `32663557182` passed quality,
dependency, x86_64 Linux, x86_64 macOS, and aarch64 macOS, but failed aarch64
Linux in the same-type revalidation evidence. That fixture unlinked and
immediately recreated a regular file; the runner legitimately reused the same
inode, making the replacement equal under the protocol's portable
`(device, inode, exact type)` identity. This was an evidence nondeterminism, not
a production failure: the other 177 private tests in that job passed, every
other CI job passed, and exact benchmark workflow `32663557187` passed both
jobs with two nonexpired artifacts bound to the seal SHA.

Test-only remediation `c6744ab5416fc4bde330d09f59dd507bd9991d72`
retains an open handle to the original regular file while unlinking and
recreating the same-type replacement. The original inode therefore cannot be
recycled before final revalidation. No production, public API, Cargo, CLI, or
contract behavior changed from the cycle-5-reviewed candidate.

Exact remediation tree `2ac83ee846e1b74b5e103f8fabcb14d828024c89`
passes the complete replacement gate. Rust/Cargo 1.94.1 formatting, workspace
warnings-denied Clippy, workspace tests, two doctests, 29/30 private, 21 direct
including hostile umask, five engine, seven host, one core, and 740/790
discovery with zero benchmarks are green. Python 130 with eight expected skips,
pinned-fx compatibility, cargo-deny 0.20.2, cargo-audit 0.22.2 over 1,225
advisories and 175 dependencies, Linux/FreeBSD/WASI and active Node 1/1, and
documentation 64/445/295/0 pass. The clean delivered-base diff covers 16 files
with 6,766 insertions and 62 deletions, adds zero unsafe Rust, and changes no
Cargo or CLI path. Optional all-feature Linux cross remains blocked only by the
host C sysroot in `aws-lc-sys`, before product Rust. A fresh locked 319,152-byte
arm64 Mach-O release CLI has SHA-256
`d5e91bac9cf07f389b98341ed0532d54d666f8aff2b92ffbd01f4a65cdfd8751`
and passes exact bare/help/human-status/JSON-status smoke with empty stderr.
This test-evidence change required three fresh exact-candidate reviewers before
a replacement seal could be pushed; its documentation-only record did not.

Formal cycle 6 reviewed exact tree-identical candidate
`9e817beb92b14ce718c9c6a2b35637fb6fa2cf7e`, tree
`d63a92fd606ac14467eda1e1d86d2f6980547176`. Correctness/API,
filesystem/robustness, and performance/concurrency all report **GREEN** with
zero findings from clean detached worktrees. Production and public integration
content remain byte-identical to the cycle-5-reviewed behavior.

Correctness/API and filesystem/robustness each stress-ran the repaired same-type
identity case 500/500 times. Both confirmed that the owned original handle spans
unlink, replacement creation, and final revalidation, then closes by RAII.
Performance/concurrency passed 64 parallel focused invocations and confirmed
one bounded per-fixture handle, no shared state, no leak on success or unwind,
and zero production runtime overhead. The reviewers also reran exact Rust 1.94.1
formatting, warnings-denied Clippy, workspace tests/doctests, 29/30 private, 21
direct, five engine, seven host, one core, Linux no-default, FreeBSD, WASI, and
active Node evidence applicable to their tracks. All historical production and
evidence findings remain closed.

Replacement documentation seal
`fe56f4c57ef18f87c742340a6060dc56b91f00f9` passed exact feature CI
`32665295323` across all six jobs. Exact feature benchmark workflow
`32665295321` passed both jobs and retains two nonexpired artifacts bound to the
seal SHA. `main` was fast-forwarded without force from exact prior main
`719a9bded86fd7ce394d482798b9064c736f43ab` to the replacement seal. Exact
main CI `32665564381` passed all six jobs; exact main benchmark workflow
`32665564382` passed both jobs and retains two nonexpired exact-SHA artifacts.
This completes delivery of native `delete_file` as the twenty-second bounded
Milestone 03 slice.

The remaining Milestone 03 native tools, top-level CLI ownership, and composed
end-to-end boundary remain pending, so Milestone 03 is not complete. This final
delivery record is documentation-only and exempt from adversarial review under
the user's instruction. Its own exact feature CI and benchmark workflows and
exact `main` CI and benchmark workflows remain required after push and cannot
be self-recorded. This record makes no product-performance, fx-equivalence, or
compatibility-status promotion claim.

## Parallel ownership and formal review

Implementation proceeds with three non-overlapping owners:

- production owns native behavior, exports, prepared-root and reference-host
  wiring, and any core contract regression needed for the existing `Delete`
  authority;
- independent tests own direct, engine, host, portability, fault, race, bound,
  cancellation, and unsupported-target evidence; and
- documentation owns this normative contract, maintained indexes and plan
  status, and the review lineage record.

Production should use a statically dispatched evidence seam in the real
pipeline for root and intermediate opens, ordinal-aware descriptor `fstat` and
pathname `statat`, checkpoints after initial and final validation, the actual
`unlinkat` flags and outcome, the checkpoint after a real deletion, and parent
`fsync`. It must add no global mutable evidence state, dynamic dispatch in the
release path, unbounded retry, or behavior-only test fork.

After one exact composed behavior candidate passes focused and complete local
gates, three fresh agents must adversarially review that same SHA for
correctness/API, filesystem/robustness, and performance/concurrency. Every
finding is fixed and all three tracks restart on a new exact candidate until
all report **GREEN** with zero findings. Documentation-only contract, seal, and
delivery commits are exempt from another adversarial cycle under the user's
explicit instruction, but still require their exact remote workflows.

## Required independent evidence

The composed candidate must prove:

- exact exports, constants, schema/property descriptions, strict shape,
  construction and tool errors, retryability, result, `Display`, and redaction;
- exact and one-over requested-path, canonical-path, component, serialized-
  argument, and serialized-result boundaries;
- effect-free preparation, denial before any lookup, exact `Delete` authority,
  and policy/direct-execution agreement on the canonical path;
- deletion of regular files and empty directories, including hostile umask,
  Unicode and literal-backslash names, with no content read or enumeration;
- root, missing path/ancestor, nonempty directory, every symlink position,
  FIFO, socket, device, and other special rejection without blocking or outside
  sentinel changes;
- retained-root rename, replacement, and removal; complete initial/final parent
  and target identity/type revalidation; retained-parent movement outside the
  public workspace; final same-class and file-to-symlink/FIFO/socket replacement
  races with referent/sentinel preservation; hard-link/open-descriptor
  survival; and immediate pathname recreation;
- production-routed root/intermediate-open, ordinal `fstat`/`statat`,
  `unlinkat`, and parent-sync faults with exact precommit, committed, and
  ambiguous mappings;
- exact empty versus `REMOVEDIR` flags, exactly one delete call on success and
  `EINTR`, no delete retry, and cumulative 16-interruption parent-sync handling;
- cancellation at every traversal/metadata boundary, immediately after both
  validation phases, immediately before the real delete, and after the real
  delete, plus unpolled/drop and engine same-poll unknown-result recovery;
- canonical reference-host composition and retained-workspace identity;
- native Linux/macOS execution, FreeBSD/WASI compilation, active unsupported-
  target behavior, no unsafe Rust, and complete regression of the previously
  delivered workspace tools.

Focused suites have run first on the local precursor. The formal behavior
candidate repeats Rust 1.94.1 formatting, workspace all-
target/all-feature warnings-denied Clippy, workspace tests, workspace doctests,
the repository Python and pinned-compatibility checks, dependency policy and
audit, portability gates, documentation integrity, diff/no-unsafe checks, and
a freshly built locked release CLI smoke. Green local evidence alone is not
delivery: the exact reviewed SHA must pass feature workflows, fast-forward
`main`, and pass exact `main` workflows.

## Pinned fx input and deliberate differences

Pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef` was observed
to expose a required delete path and accept both a regular file and an empty
directory. Its implementation is broader and pathname-based. This observation
is compatibility reconnaissance only, not an equivalence or delivery claim.

Machine-god deliberately uses strict unknown-field rejection, smaller
independent limits, effect-free canonical preflight, exact deletion authority,
retained descriptor-relative no-follow confinement, complete bounded
revalidation, explicit non-CAS race disclosure, fixed redacted errors,
cancellation boundaries, one non-retried delete call, and explicit durability
ambiguity. It does not adopt upstream external paths or broader pathname
behavior.

This slice adds no recursion, parent or target creation, wildcard or multi-path
deletion, trash/recovery mode, secure erasure, symlink-target mutation, content
read, directory enumeration, external path, CLI change, new dependency,
non-Linux/macOS hardening, compatibility promotion, benchmark workload,
product-performance claim, or fx-equivalence claim.
