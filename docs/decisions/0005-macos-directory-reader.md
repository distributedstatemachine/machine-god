# ADR 0005: metered macOS directory refills

## Decision

Permit one additional fixed binding in `machine-god-terminal-sys`:
`read_directory_chunk(BorrowedFd, &mut [u8; 8192]) -> io::Result<usize>`.
It calls the exported libSystem `__getdirentries64` entry point exactly once.
It cannot select another operation, open a path, duplicate or close a descriptor,
allocate scratch, retry an interrupted call, or resolve a symbol dynamically.
The binding crate keeps its denied-unsafe default, with documented exceptions
only for this declaration and call. Product crates continue forbidding unsafe
Rust. Existing ADR 0003 and ADR 0004 permissions are unchanged.

The safe native reader retains an initialized 8 KiB buffer and borrows the
caller's already-open directory. Construction is inert. Each `next_name`
consumes at most one record and performs at most one refill. `is_buffer_empty`
allows the scanner to charge its existing refill budget before the attempt.
An explicit `Skipped` result represents an inode-zero record; dot entries remain
ordinary names. Neither skipped records nor interrupted calls hide a second
refill. The scanner continues to own entry, byte, native-call, cancellation,
deadline and incomplete-result accounting.

## Alternatives

The pinned rustix 1.1.4 macOS `Dir` uses `fdopendir` and `readdir`. Apple libc can
prime directory reads during opening, cache union-directory contents, and refill
again while skipping records during a single `readdir`. This does not expose
the exact refill boundary needed by the bounded lexical scanner. Its Linux
`RawDir` counterpart does expose that boundary.

The inspected fdf 0.9.5 raw iterator has no public inert borrowed-descriptor
constructor and exposes pointer-oriented uninitialized storage. The inspected
purestd 0.0.3 has a safe raw-refill wrapper, but supports only ARM64 macOS and
implements a broad direct-assembly syscall runtime rather than the exported
libSystem ABI. Neither supplies the required safe two-architecture capability.
No new dependency, generic syscall interface, assembly, `dlopen`, filesystem
path authority or fallback iterator is introduced by this decision.

## ABI and ownership

Apple libc's private declaration returns `size_t` and accepts `int`, `void *`,
`size_t`, and `off_t *`. The supported ARM64 and x86_64 macOS ABIs use 64-bit
size/offset values; the failure representation is `SIZE_MAX`, with the original
errno preserved. The SDK's libsystem_kernel export table provides the symbol
for both architectures. Binding the fixed export avoids hard-coded syscall
numbers and dependence on a private trap calling convention.

Only initialized, exclusively borrowed byte storage and a local initialized
offset are passed to the synchronous call. Neither pointer escapes. The
descriptor remains owned by its caller; enumeration advances its shared open
file-description offset. Callers must serialize cursor use and must not assume
that cloning a descriptor creates an independent cursor.

Returned lengths must fit the supplied buffer. Zero means EOF. A failed call's
buffer contents are discarded; `EINTR` remains nonterminal and a later attempt
must be separately charged. Other failures and malformed records terminate the
reader. The kernel may place status flags at the buffer's end outside its
returned byte count. The reader ignores those flags and never parses beyond the
returned count; observing EOF costs its own refill rather than inferring it.

Native parses bytes without struct casts, transmutation or unsafe references.
The verified 64-bit directory layout has inode at offset 0, record length at 16,
name length at 18, type at 20 and name at 21. Bounded record lengths, complete
headers, name lengths, component bytes and exact NUL termination are checked
before allocation. Names are raw bytes, not assumed UTF-8; policy-specific
character checks and descriptor-relative metadata lookup remain scanner work.
A record is a hint from a changing directory, not a stable object or permission
grant. No file type or child identity is inferred from its spelling alone.

One kernel call may block or perform internal filesystem work. The refill cap
bounds calls and retained userspace bytes, not a hard wall-clock deadline or
kernel-internal iterations. Cancellation is observed at native call boundaries;
there is no detached reader or timer-based claim of forced interruption.

## Verification

Test real descriptor-backed enumeration, EOF, non-directory errors, unchanged
descriptor ownership, malformed/truncated records, non-UTF-8 names, zero-inode
records and exact buffer/refill boundaries. Inject `EINTR` deterministically to
prove original error propagation and separately charged retry without relying
on a timing-sensitive signal. SDK C layout assertions and fixed-symbol link
checks cover both ARM64 and x86_64; Rust tests exercise the actual host binding.
These component checks do not waive full-feature adversarial review, native
platform execution, exact CI or the scanner's composed budget tests.

Primary references:

- [Apple libc declaration](https://github.com/apple-oss-distributions/Libc/blob/main/gen/FreeBSD/telldir.h)
- [Apple libc refill and skip behavior](https://github.com/apple-oss-distributions/Libc/blob/main/gen/FreeBSD/readdir.c)
- [Apple libc descriptor opening and union caching](https://github.com/apple-oss-distributions/Libc/blob/main/gen/FreeBSD/opendir.c)
- [Apple directory record layout](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/dirent.h)
- [Apple returned-byte and status-flag behavior](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/vfs/vfs_syscalls.c)
