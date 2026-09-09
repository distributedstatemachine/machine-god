# Top-level replay command

The `replay` command deterministically renders a pinned-fx-compatible `FXTP`
terminal tape without sleeping for recorded timing or invoking an agent. It is
an offline terminal-artifact command, not the native session lifecycle's
provider-neutral record snapshot. Current delivery state and gate evidence
remain only in the
[implementation plan](implementation-plan.md#current-delivery-state); this page
defines durable behavior.

## Grammar, help, and exits

The accepted command family is:

```text
machine-god replay <tape> [--frames] [--json] [--golden <path>] [--frames-dir <path>]
```

Recognized options may occur before or after the one tape operand. Repeating
`--frames` or `--json` is idempotent. Repeating `--golden` or `--frames-dir`
uses the last supplied value. A path-valued option consumes the following token
unconditionally, including a flag-looking token. Exact recognized options are
processed before positional operands; any other token beginning with `--` is
an unknown flag, and a second ordinary positional is an error. `--` is not an
end-of-options delimiter. An empty or single-dash path is syntactically valid,
while a path beginning with `--` cannot be represented by this grammar.

An exact `--help` or `-h` anywhere after `replay` preempts parsing and every
filesystem effect, writes replay-specific help, and exits `0`. Other replay
parse errors use the pinned command's exit `1`, not the global invalid-command
exit `2`. If an exact `--json` occurs anywhere in the raw replay arguments,
including where a path-valued option would consume it, a parse failure writes
one compact JSON error to standard output and nothing to standard error.
Otherwise it writes the command-specific human diagnostic to standard error.
The stable parse codes are `MissingTapePath`, `TooManyArgs`, `UnknownFlag`,
`MissingGoldenPath`, and `MissingFramesDirPath`.

Successful replay exits `0`. A handled open, bounded-read, header, grid,
resource, cancellation, directory, artifact, manifest, or golden failure exits
`1`. Human failures use the `machine-god replay:` prefix on standard error.
JSON failures fix key order `kind,error,code`, write one LF-terminated compact
object to standard output, and leave standard error empty. Diagnostics retain
the pinned replay failure category and stable code but do not reflect arbitrary
path values or raw operating-system diagnostics. A process output failure uses
the global `machine-god: failed to write output\n` diagnostic.

## FXTP version 1

Replay accepts this byte-exact little-endian header:

```text
"FXTP\x01"       five-byte magic and format version
cols             u16
rows             u16
epoch_ms         i64
version_length   u8
version          version_length bytes
```

A tape shorter than the 18-byte fixed header fails with `TapeTooShort`, even
when its available magic bytes are wrong. A fixed-or-longer tape with the wrong
magic fails with `BadTapeMagic`, before inspecting its declared version. A
valid-magic header whose declared version extends beyond the available bytes
fails with `TruncatedVersion`. Each stable code has the redacted native message
`bad tape: <Code>`. Zero dimensions, over-bound dimensions, invalid grid size,
or an invalid terminal stream remain `BadTape`. The complete tape file must be
strictly smaller than 64 MiB; a file of exactly 64 MiB is over the limit.

The version field is arbitrary bytes. Structured output encodes it as a JSON
string when it is valid UTF-8 and otherwise as a numeric array containing each
byte value.

Every following frame has this shape:

```text
delta_ms         i32
kind             u8
payload_length   u32
payload          payload_length bytes
```

Kinds `1` through `5` are named `stdout`, `stdin`, `resize`, `sigint`, and
`marker`; every other byte is named `unknown`. All complete frames count and
retain their signed delta and payload length. Only stdout payloads feed the
terminal. Resize payloads of at least four bytes use their first two
little-endian `u16` values as columns and rows; shorter resize payloads are
no-ops and do not increment `resize_count`. Stdin, signal, marker, and unknown
payloads never change the grid.

A final frame with only 1 through 8 header bytes, or with fewer payload bytes
than its declared length, is deliberately ignored after replaying all preceding
complete frames. Replay still succeeds and writes exactly
`machine-god replay: ignored incomplete final tape frame\n` to standard error.
This warning remains standard error even in JSON mode. Header/version
truncation is not recoverable.

## Terminal rendering

The replay grid is a safe, bounded Rust implementation of the pinned fx
journal-replay terminal semantics. Parser state survives stdout frame and
internal feed-chunk boundaries. It implements cursor movement and save/restore,
autowrap, origin and insert modes, scrolling regions, line and display erasure,
character and line insertion/deletion, tabs, normal and alternate screens,
cursor visibility, synchronized-update buffering, control-string suppression,
and the pinned Unicode 17 display-unit policy for wide glyphs, combining
suffixes, variation selectors, emoji sequences, and invalid or fragmented
UTF-8. SGR and OSC 8 presentation state never leaks escape bytes into a plain
snapshot.

Resize keeps the top-left visible cells, clips cells outside the new bounds,
fills growth with blanks, repairs wide-cell continuations, and resets the
scroll region and origin exactly as the pinned replay grid does. A snapshot
contains every row and trailing blank display column in this shape:

```text
|row contents and trailing blanks|
```

Each row ends with LF. Wide continuation cells produce no bytes. Replay has no
scrollback and never emits styling or hyperlinks in a snapshot.

## Output modes

With no output option, standard output is the final grid snapshot. `--frames`
writes a header and complete grid after every non-marker frame:

```text

--- frame 2 (stdout, +7ms) ---
|grid|
```

Markers still count toward the one-based frame number. Stdin, signal, short
resize, and unknown frames therefore produce unchanged snapshots in frame
mode. `--json` writes a compact LF-terminated summary with exact key order
`cols,rows,epoch_ms,version,frames,frame_count,resize_count,stdout_bytes`.
Every frame entry fixes key order `delta_ms,kind,len`. Summary `cols` and `rows`
are the initial header dimensions even after resize; counters use checked
arithmetic.

Options compose in pinned order. Frame snapshots are written before the JSON
summary. `--golden` writes the final grid to its exact path with create-or-
truncate behavior and suppresses only the ordinary final-grid output; it does
not suppress frame or JSON output. `--frames-dir` suppresses nothing.
Consequently `--frames --json` intentionally produces frame text followed by
JSON rather than a pure JSON stream.

All process output is assembled within a checked 128 MiB ceiling before the
CLI publishes it. Default and JSON replay are O(tape bytes plus grid cells);
frame output is inherently O(complete frames times grid cells). Crossing an
output, grid, Unicode-pool, synchronized-update, frame, counter, or artifact
bound fails with `ResourceLimit` rather than wrapping or allocating without a
limit.

## Golden and frame artifacts

`--golden <path>` does not create parent directories. On success it creates or
truncates the exact file and writes the final snapshot bytes.

`--frames-dir <root>` recursively creates `<root>` and `<root>/frames`, then
writes artifacts for every complete frame, including marker and unknown kinds:

```text
<root>/manifest.json
<root>/frames/0001.json
<root>/frames/0001.grid.txt
```

The decimal index has a minimum width of four and is not truncated above
`9999`. Existing current artifact names and the manifest are truncated;
unrelated files and stale higher-numbered frames remain. The command does not
promise rollback after visible artifact writes, so an I/O failure may leave a
partial tree.

Per-frame JSON fixes key order
`index,delta_ms,elapsed_ms,kind,payload_len,size,cursor,footer_candidates,visible_markers`.
Size contains `cols,rows`; cursor contains one-based `row,col` and `visible`.
Footer candidates identify a prompt-like row between divider-like rows using
zero-based visible-row indices. Visible markers retain encounter order and
include each nonempty marker payload currently present in the plain grid. Like
the version field, an emitted marker payload is a JSON string when it is valid
UTF-8 and otherwise a numeric array containing each byte value.
`manifest.json` fixes key order
`cols,rows,epoch_ms,version,frame_count,resize_count,stdout_bytes,frames_dir`,
where `frames_dir` is the literal string `frames`.

Frame-artifact mode accepts at most 4,096 complete frames and 128 MiB of
generated artifact bytes. These explicit repository resource ceilings contain
the number of created files even though the 64 MiB tape format can encode far
more zero-payload frames.

## Authority, cancellation, and compatibility

Replay uses only the explicitly supplied tape, golden, and artifact paths. It
does not read machine-god configuration or state, discover credentials, inspect
a workspace, load a durable session, construct an engine/provider/runtime,
prompt, invoke a tool, contact a network, or require a TTY. The future is inert
until first poll. Its bounded local file and terminal work is synchronous on
the polling thread, with cancellation checks before the first effect, between
complete frames, and between bounded stdout feed chunks. No background task or
external effect survives return.

The command implements the pinned fx replay scenario and FXTP v1 artifact
formats in Rust. Zig remains only an upstream benchmark/evidence build input;
the machine-god binary never invokes or embeds a Zig runtime. The separate
native session lifecycle method named `replay` continues to return an owned
provider-neutral record to Rust callers and is not used by this CLI command.

## Native recording compatibility

`TerminalTapeRecorder` writes FXTP v1 from explicitly supplied native authority.
It is a native backend, not a process-global recorder: selecting CLI flags or
environment values and wiring the actual terminal streams are host duties.
The native API reads no environment variables, discovers no workspace, and
creates no conversation or provider effects. The host must await successful
requested startup before admitting a conversation, and report startup failure
instead of silently continuing without the requested recording.

An automatic destination takes the selected `Arc<FileSessionStore>` and its
logical state-root label. Startup clones that retained root descriptor on its
owned worker and creates `recordings/machine-god-record-<epoch>-<random>.fxtape`.
The directory must be owner-private, the newly created file is owner-private,
and creation is exclusive with eight bounded name-collision attempts. The
logical path is reporting metadata, not authority to reopen a renamed root.
An explicit absolute destination similarly uses exclusive private creation;
it never truncates an existing tape. Explicit-path parent directories must
already exist and are walked descriptor-relatively without following symlinks.
Neither mode follows a final symlink. This no-overwrite rule intentionally
differs from pinned fx's explicit-path truncation. The interactive host
distinguishes required flag startup from optional environment-only startup as
described below. Confined recording is
implemented on Linux and macOS, alongside the native worker-scope runtime.
The request's effect-free `validate` method remains available on other platforms
and returns the explicit `UnsupportedPlatform` error before native admission.

The injected header supplies initial dimensions, epoch milliseconds and at most
255 version bytes. Its one-byte length exactly matches emitted bytes; an
overlong version is rejected rather than emitting a malformed header. Each
frame receives an injected timestamp. Delta arithmetic saturates before
clamping to `0..=i32::MAX`, including backward timestamps and signed extremes.
Stdout frames contain only the accepted prefix supplied by the real terminal
writer. Accepted stdin is excluded unless the request explicitly opts in;
resize, SIGINT and marker frames use the same FXTP kinds as pinned fx. Empty
stdout/stdin writes produce no frame. The recorder never writes to the terminal
or records an attempted-but-unwritten suffix on the caller's behalf.

One persistent file-owning worker is enrolled in `NativeOwnedWorkerScope`.
Requests use a one-slot nonblocking channel and asynchronous receipts, with no
thread or unbounded queue per frame. At most one 64 KiB payload is executing and
one is queued; an admission attempt may transiently hold one additional bounded
payload. `Busy` admits nothing, so the host retains the accepted bytes and
retries while continuing its input, signal and native-owner loop. Request futures
are inert until polled. Dropping an admitted receipt cannot cancel or detach
its write. Dropping the recorder disconnects admission; queued writes and final
flush/sync/close remain on the enrolled worker.

The hard limits are 64 KiB per payload, 1,000,000 complete frames and 64 MiB minus
one byte per tape, with smaller explicit file/frame bounds supported. Each
header or payload write has a 4,096-attempt limit, including interrupted and
short writes. Limits, zero-progress writes, I/O errors and observed cancellation
are explicit failures, not silent truncation. Failed recording retains the first
failure plus acknowledged byte and complete-frame counts; a torn final frame is
not counted. Complete preceding frames remain available to replay, whose
existing incomplete-tail rule handles a partially written final frame.

`finish` acknowledges actual flush/sync attempts and file close. Its status is
complete only when explicitly finalized without an earlier failure; abandonment
remains incomplete even if cleanup succeeds. The final receipt remains awaitable
after a failure disconnects admission or a prior finish future is dropped; no
new worker is required to finish an already closing host. Observation-only completion handles
survive recorder drop without keeping admission alive. A file-close receipt is
not a thread-join receipt: the host closes its worker scope and observes its
existing collector-completion fence before declaring shutdown complete. This
also covers dropped startup/operation responses and thread-local cleanup.

## Interactive recording startup

One terminal `--record` modifier requests recording for interactive startup or
an interactive resume, including the picker and latest/exact aliases. Leading
workspace modifiers remain before the command; `--record` remains the final
token. It is rejected for administrative commands, `ask`, and a resume carrying
a one-shot prompt, before host effects. For example:

```text
machine-god --record
machine-god --add-dir ../shared --record
machine-god -r --record
machine-god --continue --record
machine-god resume --id <session-id> --record
```

Validated interactive startup captures the compatibility variables `FX_RECORD`
and `FX_RECORD_INPUT` once. No recording variables are read by noninteractive
commands or by the native recorder. A nonblank `FX_RECORD` selects an explicit
destination, including when no flag is present; otherwise `--record` selects the
automatic retained-store destination. Paths are trimmed only of ASCII space,
tab, CR and LF, retain non-UTF-8 bytes, and resolve relative to the captured
working directory without shell expansion. The resolved absolute path is
bounded to 4,096 bytes and remains subject to native no-symlink/no-overwrite
validation. Parent-directory components are walked through retained directory
descriptors: `real/../tape` works, while `missing/../tape` and
`symlink/../tape` fail instead of skipping those prefixes. Automatic state labels
remain normalized. Automatic recording uses the selected native state authority, not
an independently discovered home or temporary-directory fallback.

Stdin recording is off by default. `FX_RECORD_INPUT`, after the same trimming,
enables it only for case-insensitive `1`, `true` or `on`. This is an explicit
privacy choice: accepted input can include prompts and approval answers, while
recorded stdout can include sensitive conversation content. The initial notice
shows the escaped tape path and whether stdin is included; it reports active
capture, not successful finalization.

Initial terminal dimensions, epoch milliseconds and the binary version are
injected into the native request. Tape startup succeeds before a conversation
is created, resumed or admitted through the picker. A failed `--record` startup
is fatal. A failed environment-only startup closes and joins its attempted
recording scope before continuing with the fixed `recording unavailable`
notice; it never claims active recording. A signal observed during recording
startup cancels and joins that setup and prevents conversation admission,
including for an optional environment-only request.

The output bridge retains each actual accepted stdout prefix until its tape
receipt succeeds or reports an explicit failure, splitting payloads at the
native 64 KiB frame bound. The presentation lane holds at most eight queued
events; opted-in stdin is copied once at native chunk receipt. Resize, SIGINT
and the explicit `machine-god:interactive` startup marker use their respective
FXTP frame kinds. Saturation, clock failure and native write failure make a
tape incomplete rather than silently dropping captured events.
Stdout timestamps are captured at the actual accepted-write boundary, not when
the presentation later consumes a receipt: the last successful write's time
survives subsequent write errors and acknowledgement/tape backpressure. Raw
stdin timestamps are captured when its native chunk is observed, not when a
later parser consumes a retained remainder. Once final tape close is requested,
event admission ends; later signals still affect exit without reopening the tape.

Input and native conversation/terminal cleanup progress independently of tape
acknowledgements and blocked stdout. Final presentation still records accepted
output, then requests tape flush/sync/close. The outer CLI worker closes and
joins the separate recording worker scope after all presentation owners drop.
On a latched signal, output and tape finalization share one 100 ms deadline
starting when the post-cleanup presentation is first polled, not when the signal
arrived. Neither output nor tape acknowledgements restart that deadline.
Signals remain latched until this actual join; neither a final tape receipt nor
an output acknowledgement authorizes early process exit. Abandonment is never
reported as a complete tape. These CLI rules retain the pinned environment
selection and stdin opt-in semantics from
[`record_tape.zig`](https://github.com/vercel-labs/fx/blob/b1774fbf6c7602b503026f96f6e960e946c692ef/src/core/workspace/record_tape.zig),
with the explicit native authority and no-overwrite differences above.
