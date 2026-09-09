# Native clipboard process capability

`NativeClipboard` receives an explicit `NativeClipboardExecutable` (absolute
path plus retained executable file), absolute working directory, frozen
environment, and `NativeOwnedWorkerScope`. Construction is inert. Native code
does not discover PATH, display services, locale, cwd, or session text.
The host supplies the actual pbcopy executable on macOS or xclip executable on
Linux, not a script or wrapper. Arguments are fixed: none for pbcopy;
`-selection clipboard` for xclip. The backend constructs no shell command or
configurable argv, and offers no OSC52 or alternate clipboard backend. This
trusted-executable capability is not a general-purpose interpreter-free
launcher: `std::process` retains platform executable-format semantics, including
possible shell interpretation of an invalid executable image. Native code does
not attempt to infer executable validity from header bytes.

`copy(Arc<str>, CancellationToken)` is inert until polled and accepts up to
`MAX_FILE_SESSION_BYTES` (8,651,165) raw UTF-8 bytes. It writes those exact bytes
through an exclusively owned nonblocking stdin pipe, in chunks no larger than
4096 bytes, then closes stdin. No newline or terminator is appended. Only a
positively observed normal direct-child exit code zero is success. Successful
xclip may leave a clipboard owner running; success does not kill that service.

One operation is admitted per shared backend allocation. Admission remains
held through actual child reaping, including deferred cleanup, not merely
response readiness. Clones cannot overlap a cancelled but unreaped helper.
Operations have an independent ten-second deadline. Cancellation, abandonment,
write failure, and timeout close stdin and stop the owned direct child. A
private cancellation token handles future drop without cancelling the supplied
caller token. Failure never promises that the clipboard remained unchanged or
that a partial clipboard effect was rolled back.

The injected scope owns the worker and any quarantined reap obligation. A
response is not a worker-join receipt. Hosts cancel their jobs, close the scope,
then await its actual completion on a dedicated worker; a cleanup grace expiry
never becomes successful cleanup or copy. Clipboard admission is not permission
to acquire session/model state: the caller selects and retains the source text.

Paths are bounded at 4096 bytes. Frozen environments use the terminal limits:
512 entries, 1024 bytes per key, 16 KiB per value, 256 KiB aggregate. Duplicate
keys, NUL, invalid key syntax, and relative or parent-relative paths reject.
Executable file identity/type/mode/timestamps are checked against the supplied
spelling on the worker before spawn. The host must reserve that installation
against replacement; the check and path-based spawn are not claimed atomic.
Cwd lookup is performed by process launch, not by the inert constructor.

Tests use injected fixture children and never modify the user's clipboard.
