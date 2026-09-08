# Native interactive byte input

The Linux/macOS `NativeInteractiveInput` component receives explicit descriptor
authority and returns raw chunks of at most 4,096 bytes. It does not select ambient
stdin, parse lines or UTF-8, edit terminal settings, interpret slash commands,
or route human answers. Those framing and presentation decisions belong to the
CLI. Conversation and transition ownership remains native.

## Descriptor and status-flag authority

`Disabled` is the inert default. `PreserveNonblocking(File)` requires existing
`O_NONBLOCK` and never changes status flags. `AdoptNonblockingStatus(File)`
explicitly authorizes setting that flag on the shared open-file description,
including every duplicated alias. Other observed flags are preserved, and flags
are never restored on drop. Owning or duplicating a `File` does not establish
exclusive open-file-description ownership.

The caller reserves input consumption, termios and status-flag control for the
adapter's lifetime; another alias must not clear `O_NONBLOCK` or change terminal
settings. Explicit adoption is not
automatically a safe ordinary-CLI stdin handoff: it can change the caller's or
shell's shared terminal flags. The CLI must separately establish appropriate
authority rather than silently adopting or restoring those flags.

`PreserveShared { input, helper }` accepts explicitly supplied shared stdin
without changing its status flags. On first poll, a verified TTY is reopened by
its actual terminal name with a fresh nonblocking open-file description; device
identity and unchanged termios/source flags are checked before use. PTY master
clone descriptors are rejected rather than reopened as a different terminal.
Descriptor aliases such as `/dev/fd` are not independent terminal acquisition. Terminal
settings and input consumption remain caller-reserved; the adapter never changes
or restores them. Already-nonblocking pipes use direct reads. Blocking pipes use
an owned helper process, so a competing reader cannot strand a blocking native
thread after consuming readiness. No Linux `/proc` availability is assumed.

`NativeInteractiveInputHelper::new(program, executable)` binds a bounded absolute
program spelling to a retained file without opening or executing it. First-poll
helper admission checks the named executable against that retained identity.
The caller reserves the helper installation against replacement during use;
this is not atomic descriptor-based execution. Only the exact private
`INTERACTIVE_INPUT_HELPER_ARGUMENT` dispatch calls
`run_interactive_input_helper`, before ordinary CLI/provider initialization.
The supplied pipe becomes helper stdin; an inherited private duplex socket is
its control/result channel. The helper receives no ordinary environment or
output stream. It validates its descriptors and handshake before consuming
input, and one credit permits one read of at most 4,096 bytes. Results have fixed
bounded framing; malformed frames are fixed errors. This library contract does
not by itself install the private dispatch in a CLI binary.

Only readable FIFO/pipe descriptors and character devices proven to be TTYs are
accepted. Other devices, regular files, directories, and sockets are rejected
before reading or changing flags. There is no cancellation guarantee for
arbitrary regular-file or network-filesystem reads because those inputs are not
supported. Descriptor numbers, paths and input content are excluded from errors
and debug output.

## Admission, bounds and settlement

Construction performs no descriptor inspection, flag mutation, read, worker
admission, or ambient input acquisition. First polling admits one worker through
the existing bounded native collector, in a dedicated per-adapter scope. There
is one fixed-size chunk slot. A pending poll authorizes one chunk; consuming it
does not authorize another read until the next poll. Native does not accumulate
lines or prefetch another chunk while the slot is occupied.

Bytes are unchanged, including delimiters and partial code points. A zero-byte
pipe or canonical-terminal read reports EOF. For a verified noncanonical TTY
with `VMIN=0`, zero means no data: the adapter keeps waiting with bounded backoff,
including after peer closure if there is no positive terminal-end evidence.
Such input remains cancellable; it never manufactures EOF. Terminal read/poll
failures remain failures rather than guessed clean EOF. Errors are fixed and
emitted once, followed by exhaustion.
Cancellation discards unconsumed input and has a separate terminal-outcome slot,
so it does not depend on input consumption or stdout acknowledgements.

Linux TTYs and directly held pipes use finite readiness polling followed by
nonblocking reads.
Verified macOS TTYs use nonblocking reads with a condition-variable wait on
`EAGAIN`: pinned rustix documents that `poll` does not support `/dev/tty`, while
its `select` alternative requires unsafe code. Readiness/condition waits are at
most 25 ms per iteration. `EINTR` retries and `EAGAIN` yields; arbitrary TTY
`EIO` is a read failure, not manufactured EOF. These are cooperative observation
bounds, not hard wall-clock scheduling guarantees.

`request_stop` and drop stop only this adapter, never cancel its injected shared
host token. Host cancellation also ends idle or backpressured input without
requiring another consumer poll. The worker retains actual descriptor ownership
until it exits. `completion()` observes scope closure and the collector's actual
thread join, including destructors, not merely an EOF/cancellation response.
Shared blocking-pipe cancellation first closes the private channel and requests
exact-child termination/reaping. If immediate reaping is deferred, the existing
owned quarantine retains the input scope through actual reap; completion stays
pending. There is no detached helper or hard cleanup deadline promise. Full
input slots and stalled handshakes do not prevent stop/reap progress.
An unpolled descriptor remains owned by the adapter until it is dropped or a
cancelled poll consumes it. Completion handles carry no admission authority.

The CLI must keep presentation and native progress independent: at most one
bounded stdout chunk may await acknowledgement while cancellation, committed
transitions and shutdown continue being polled. Obsolete text may be discarded
during draining, but transition receipts and finalization errors must survive.
This byte adapter does not itself implement that interactive owner or driver.

## Explicit raw-terminal lifetime


`NativeInteractiveTerminal::new(tty)` receives an explicitly owned descriptor and
the caller's exclusive authority to change that terminal's termios. Construction
and unpolled activation/restoration futures are inert. First activation polling
admits one scoped native worker, rejects non-TTY input before mutation, snapshots
original settings, installs raw mode and verifies it on the same retained file.
No file status flag is changed. Owning or duplicating a file does not prove
exclusive terminal-mode authority; no other alias may change settings during the
guard's lifetime.

The raw settings follow pinned `shell_runtime.zig`: clear `BRKINT`, `ICRNL`,
`INPCK`, `ISTRIP`, `IXON`, `IXOFF`, `ECHO`, `ICANON`, `IEXTEN` and `ISIG`; select
`CS8`; set `VMIN=1`, `VTIME=0`; preserve other settings, including output modes.
Ctrl-D is then raw byte 4, not a canonical-reader EOF. Composer editing and
active-turn decisions remain the UI's responsibility.

The lifetime is single-use: restoration closes it permanently. An unpolled
activation can be dropped without effects; dropping a started activation
requests restoration even if raw-mode publication was not consumed. Explicit
`restore()` returns `NotActivated` only when no mutation was attempted, or
`Restored` after original settings are installed and verified. Failed or
uncertain restoration stays an error on repeated calls. Drop requests best-effort
owned cleanup but does not assert success. `completion()` observes actual worker
joining, independently of receipt consumption; completion alone is not evidence
that restoration succeeded.

The UI must stop and join its input adapter before restoring/dropping this guard,
then observe restoration and guard completion before final output. There is no
automatic restoration tied to a shared host token. Like the pinned terminal,
restoration uses flush semantics, discarding pending terminal input. Native
terminal syscalls are not assigned fictional hard deadlines; the retained worker
owns settlement even after the awaiting future is dropped. This component does
not itself implement a cursor composer. The CLI composes its raw-input lifetime
with the native input reader and observes both joins before final presentation.

## Read-only presentation dimensions and display units

`NativeInteractiveTerminalSizeReader::new(stdout_tty)` binds an explicit retained
TTY without ambient lookup or effects. `read_columns()` is inert until polled;
each admitted read verifies the character-device TTY and returns its actual
nonzero column count. There is at most one native read, including a read whose
future was dropped. Zero columns, invalid descriptors and unavailable reads
are fixed errors, not guessed defaults. Closing/dropping the reader closes
admission; its completion observes actual admitted-worker joining. This reader
does not install resize handlers or change status flags or termios.

`native_terminal_display_unit_at(text, byte_offset)` is a pure projection of the
same pinned Unicode tables used by native screens. It rejects offsets outside
UTF-8 boundaries and end-of-input, otherwise reports byte length and cell width.
It is not a generic grapheme algorithm; following zero-width marks may be
separate units. The CLI composer and viewport share this algorithm rather than
maintaining independent Unicode tables.
