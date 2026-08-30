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
