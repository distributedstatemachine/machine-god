# Performance

Machine-god treats resource bounds, regression evidence, and comparative
performance claims as different things. Current claim eligibility and milestone
state live only in the
[implementation plan](implementation-plan.md#current-delivery-state).

## Evidence classes

| Class | What it establishes | What it does not establish |
| --- | --- | --- |
| Structural bound | A code path limits retained bytes, counts, work steps, concurrency, or deadlines | Speed, low memory use, or superiority to another program |
| Regression evidence | A particular build and scenario completed with recorded output, size, or timing | Scenario equivalence or a stable product-performance claim |
| Bootstrap harness evidence | The pinned checkout, build, isolation, sampling, and artifact path work | Comparable end-to-end agent behavior |
| Claim-eligible comparison | Equivalent workloads, retained provenance and raw samples, reviewed statistics, and enforced thresholds | Performance outside the measured environment and scenarios |

Passing CI or producing a release binary is delivery evidence, not by itself a
performance result. Unimplemented or behaviorally different fx scenarios remain
non-equivalent, unmeasured, and claim-ineligible.

## Measurement protocol

A comparative result must:

1. use the exact upstream repository and commit in
   `benchmarks/upstream.lock`;
2. build both programs in their intended optimized release modes from clean,
   provenance-recorded inputs;
3. run on the same dedicated hardware, operating system, architecture, power
   policy, and filesystem;
4. isolate configuration, state, caches, temporary files, and network effects;
5. prove that both sides perform the same observable scenario before comparing
   timings;
6. warm up before at least 30 measured runs and retain every raw sample;
7. report median, p95, confidence intervals, peak RSS, executable size, and
   relevant binary sections; and
8. store command lines, environment allowlists, input/output digests, toolchain
   versions, commit IDs, and artifact digests with the result.

Outlier handling and failed samples must be specified before measurement.
Results may not discard safety checks, resource limits, permission boundaries,
or durability work to improve a number. The release thresholds are owned by the
implementation plan and should not be copied into mutable overview pages.

## Pinned upstream harness

`benchmarks/upstream.py` prepares a fresh pinned fx checkout, builds the
upstream binary, builds machine-god from materialized source, and validates a
provenance-complete evidence object. `benchmarks/run.py` collects the local
bootstrap record; `benchmarks/check.py` validates its schema and classification.
CI uploads the resulting JSON as short-lived evidence.

`benchmarks/with_zig.py` provides the exact checksum-pinned upstream build tool
in a private operating-system temporary directory on supported Linux and macOS
hosts. Keeping the toolchain outside the checkout prevents its large ignored
library tree from entering repository-cleanliness scans. Each run re-hashes the
retained official archive, extracts a fresh toolchain, passes that executable
explicitly to the harness, and removes the extraction on success or failure.
The wrapper forwards `SIGHUP`, `SIGINT`, and `SIGTERM` to the active child
process group, whose numeric identity remains reserved by a readiness-handshaked,
unreaped private anchor through final group signaling. The first handled signal
is latched until the command has joined that group, so anchor startup cannot
consume the command's one graceful delivery. Parent mask restoration is
independently bounded. Readiness-descriptor ownership is detached before one
close attempt, because an interrupted close can already have released the
numeric descriptor and retrying it could close unrelated reused authority. An
ambiguous close fails the current launch, while a persistent mask-restoration
failure poisons further launches; neither replaces an active launch exception.
Cleanup after every post-launch exception defers handled signals, skips unused
grace time once the command exits, retries final group termination, and retains
identity-safe direct-child kill and reap fallbacks before unwinding the extraction
context. Readiness failure is observed through the pipe without reaping the
anchor before its final group signal. Unless group-wide termination and both
direct-child reaps are confirmed, an otherwise successful collection fails
closed instead of emitting clean evidence. The upstream harness converts the
first forwarded signal into structured unwinding. On Linux it defers delivery
from process creation through
containment attachment, so independently grouped build or sample processes and
their descendants are owned and reaped before the harness exits.
Linux process-table discovery stops after 65,536 numeric entries, builds one
parent-to-children index, and traverses at most 4,096 contained identities per
refresh; overflow is retained as owned cleanup work and fails closed instead of
turning cleanup into an unbounded ancestry rescan. An overflow also retains an
explicit first-unowned identity witness until a later complete refresh clears
it. The independent process-table cap likewise retains the first unscanned PID
as a conservative witness. Cleanup that reaches its deadline returns the
applicable witness as incomplete rather than reporting a clean process tree.
The gated measurement launch owns both pipes and the forked child as one
transaction; any setup exception closes the descriptors and boundedly kills and
reaps the child. Cleanup isolates failures in individual kill, pipe, wait, and
supervisor operations, defers handled termination through descriptor detachment
and closure, and surfaces the latched signal only after containment cleanup
finishes.
Separate bounded-wait coordination locks serialize archive publication and
active-run recovery, while a per-run lease prevents a live extraction from
being reclaimed. Archive validation opens one no-follow descriptor, verifies
its type, identity, and exact pinned size, hashes only those declared bytes plus
one EOF witness, and revalidates the descriptor and path afterward. Archive,
active-run, and trash namespaces have hard entry
capacities and stop scanning at a single overflow witness; partial-archive and
stale-tree recovery each process only a fixed batch. Fixed cache infrastructure
uses the same root-capacity admission rule, and cursorless recovery processes a
bounded first window so it can free capacity for a persistent fairness cursor.
When the root cannot admit that cursor, recovery stores the bounded rotation
cursor in the already-locked active coordination file, so live entries in an
early window cannot starve reclaimable later entries. Both standalone and
coordination-file cursors use two fixed-size generation/checksum slots, so a
partial, failed, or interrupted update preserves the last complete position.
Recovery acquires the
active-run lock before briefly deferring termination for ownership mutations,
so lock contention remains interruptible. Trash leases are opened without
creation and inode-revalidated before reclamation. Stale and normally completed
trees are atomically moved to private same-filesystem trash, recursively deleted
after releasing the lock, and have their lease retired under the lock. Capacity
admission reserves the two root entries needed for every live run's cleanup, so
subsequent invocations can proceed while deletion runs without unbounded cache
growth.
When post-collection provenance validation is requested, the wrapper retains
the toolchain through that check, binds the evidence and binary paths to that
same collection, and removes it afterward. The wrapper exclusively owns the
upstream `--zig` argument. It never replaces or depends on a system Zig
installation.

The current inventory includes non-equivalent scenarios plus status workloads
whose captured outputs pass strict equivalence probes. All remain regression
evidence and claim-ineligible until M07 implements the complete measurement
protocol and thresholds. Zig participates only as the upstream fx build tool;
it is not a machine-god implementation or runtime dependency.

Compatibility is scenario-based. Source layout, language, runtime, command
names, or a shared label are not enough to establish equivalence. See
[compatibility.md](compatibility.md).

## Core structural bounds

Default `EngineLimits` are replaceable with lower valid values, but JSON depth
can never exceed the independent hard ceiling of 64 containers.

| Resource | Default |
| --- | ---: |
| Model rounds per turn | 8 |
| Model events per turn | 4,096 |
| Tool calls per turn / per round | 16 / 4 |
| JSON depth / nodes | 64 / 65,536 |
| Assistant text / observer reasoning | 1 MiB / 1 MiB |
| User prompt | 256 KiB |
| Session metadata / inference options | 256 KiB / 64 KiB |
| Transcript messages / serialized bytes | 4,096 / 8 MiB |
| Cached tool catalog | 1 MiB |
| Tool arguments / one serialized result / cumulative results | 64 KiB / 64 KiB / 256 KiB |
| Permission denial reason | 4 KiB |

Validation uses checked arithmetic. JSON validation is iterative with auxiliary
memory proportional to depth and stops after the configured limit plus one;
later serialization, cloning, and destruction remain protected by the hard
depth ceiling.

## Native structural bounds

The table is a compact orientation, not a replacement for public Rust constants
or the linked normative contracts.

| Area | Principal bounds |
| --- | --- |
| [read/list](read-file.md) | `read_file` retains at most 8 KiB; `list_files` returns at most 100 names and retains at most 16 KiB of name bytes |
| [glob](glob-files.md) | 100,000 visited entries, 16 MiB name bytes, 8,388,608 matcher steps, depth 256, 100 returned paths |
| [grep](grep-files.md) | 100,000 entries, 10,000 candidates, 200 KiB per text file, 64 MiB aggregate content, separately bounded include/content match work, 48 KiB result |
| [write/edit](write-file.md) | 48 KiB content/preimage/postimage, 4 KiB paths, 256 components, 8 KiB I/O chunks, eight stage-name attempts; edit additionally caps matching at 393,216 steps |
| [copy](copy-file.md) | 16 MiB source, one reusable 64 KiB transfer buffer, 4,096 I/O calls, eight stage-name attempts |
| [session store](session-store.md) | 8,651,165 bytes per record; listing returns 100 IDs, scans 1,024 entries, and accepts 64 MiB aggregate record bytes |
| [web fetch](web-fetch.md) | 2,000-byte URL, 24 KiB body, 56 KiB result, 32 DNS addresses, 60-second total deadline, default 8/hard 32 active calls |
| [web search](web-search.md) | 4 KiB query, 256 KiB response, 64 KiB record, 256 records, 10 sources, 48 KiB result, 30-second total deadline, default 4/hard 16 active calls |
| [terminal](terminal.md) | 32 KiB command and 256 KiB environment snapshot; foreground exec: 64 KiB retained output, 1 MiB produced-output cutoff, 48 KiB result, 120-second default timeout, default 4/hard 16 active calls; background output: 16 live/100 closed streams, 64 KiB retained prefix per stream, 7 KiB read page, four active reads, and a saturating `u64` observed-byte count without the foreground cutoff; bounded final-drain exhaustion marks the count as incomplete |
| [vision](vision.md) | 20 sources, 4 KiB focus, 8 MiB per image, 64 MiB aggregate reads/admission plus one growth witness, 12-byte pre-allocation signature probe, one lazy reusable read scratch, sequential batches of 8 images/8 MiB, 20 KiB evidence per attempt, 64 KiB response, 48 KiB projected result, 60-second userspace/network deadline, default 2/hard 8 active calls |
| [questions](ask-user-question.md) | four questions, six options each, 4 KiB aggregate raw answers, 48 KiB result guard, default 1/hard 8 active prompts, 256 callbacks across one prompt lifetime |

Most native tools fail fast when their concurrency capacity is full. Capacity
ownership extends through tool-owned callbacks, workers, publication, cleanup,
and return as documented, preventing detached tails from creating unbounded new
work. A timeout covers controllable userspace phases; blocked kernel calls,
filesystem latency, advisory-lock waits, injected synchronous poll/drop, and
arbitrary Waker callbacks may exceed it.

## Allocation and streaming principles

- Parse and validate before cloning or recursive serialization.
- Stream bounded files and bodies through reusable fixed-size buffers where the
  contract permits it.
- Charge complete scans, skipped values, overflow witnesses, and matcher work,
  not only retained success data.
- Grow large trackers fallibly and in proportion to admitted work instead of
  reserving their maximum for small inputs.
- Use fail-fast bounded concurrency around network, process, and prompt work.
- Retain permits until in-flight callbacks and worker ownership actually settle.
- Keep provider wire parsing and native effect buffers out of core transcripts
  unless they become a validated provider-neutral result.

Exact allocation behavior is implementation evidence, not a stable public API.
When an allocator probe matters to a review, keep it isolated from production
dependencies and record it in the relevant historical ledger.

## Optimization rule

Profile before optimizing. Optimize the dominant claim-eligible path, rerun
correctness and lifecycle gates, and compare against the retained baseline on
the same environment. An optimization is unacceptable if it weakens authority,
redaction, cancellation, durability, or a resource bound.
