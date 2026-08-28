# Native `vision` tool

This document defines the durable Milestone 03 contract for machine-god's
locally dispatched `vision` tool. The observable input and result shapes are
derived from pinned fx revision
`b1774fbf6c7602b503026f96f6e960e946c692ef`; the smaller resource envelope and
workspace-only path authority are intentional machine-god bounds.

## Boundary

`vision` inspects authorized local raster images through the configured AI
Gateway model and returns bounded, structured factual evidence. Image bytes
remain transient native data: they never become provider-neutral content
blocks, ordinary conversation history, tool results, engine events, or durable
session records.

The exact model-visible input is an object containing `focus` and exactly one
source:

```json
{
  "paths": ["screenshots/login.png"],
  "focus": "Read the visible error and describe the failed UI state."
}
```

or:

```json
{
  "image_ids": [1],
  "focus": "Read the visible error and describe the failed UI state."
}
```

Unknown or duplicate object members, explicit `null` source members, mistyped
values, an empty source array, more than 20 source entries, duplicate entries,
a non-positive image ID, or a blank, NUL-containing, or over-4,096-byte `focus`
are invalid. Source order is significant. Preparation preserves `focus` bytes
after validating that ASCII-edge trimming is nonempty; it does not otherwise
rewrite the requested analysis.

Milestone 03 implements workspace-relative `paths`. Absolute paths, `~/`,
ASCII-whitespace-only paths, empty components, `.`, `..`, repeated separators,
NUL, overlong components, and paths outside the retained workspace identity
are rejected. Path strings are normalized before policy and execution and
stable first-seen order is preserved. Attached-image storage and prompt image
ingestion are not yet part of machine-god, so valid `image_ids` produce ordered
`image_unavailable` records without filesystem or network effects. This
retains the pinned public schema without inventing durable image state.

## Authority and execution agreement

Path preparation is synchronous, bounded, and effect-free. It returns one
first-party composite capability containing the exact normalized paths and
Gateway destination:

```json
{
  "type": "vision",
  "paths": ["screenshots/login.png"],
  "target": {
    "scheme": "https",
    "host": "ai-gateway.vercel.sh",
    "port": null
  }
}
```

This is one policy decision for one indivisible operation: disclosing those
specific workspace files to that exact remote destination. A filesystem-only
capability would hide disclosure; a network-only capability would hide which
files leave the host. Production fixes the destination to canonical
`https://ai-gateway.vercel.sh`. Custom hosts must inject the canonical HTTP(S)
target actually used by their opaque transport.

After approval, execution reparses the canonical arguments and rejects any
capability-divergent input before opening a descriptor or polling the
transport. Linux performs one whole-path `openat2` lookup beneath the retained
root with all symbolic links forbidden, then repeats that confined whole-path
lookup after reading and requires the fresh descriptor identity to match.
macOS performs one whole-path `openat` lookup with `O_NOFOLLOW_ANY` and verifies
the descriptor's exact root-relative binding before and after reading. No
user-space intermediate directory descriptor is carried into a later component
lookup. The reader then requires a regular file, fingerprints descriptor
identity, link count, type/mode, size, and modification/change timestamps, and
requires an exact match at EOF. Directories, symbolic links, FIFOs, sockets,
intermediates that no longer bind the opened file beneath the root,
disappearing, shrinking, growing, or concurrently changed files and identity or
root-validation failures become path-redacted per-image failures without
provider access for that image.

Valid `image_ids` require no policy-governed effect in this milestone and are
prepared without authority. They deterministically return the documented
unsupported records and do not consult the workspace or transport.

## Media admission and batching

The tool recognizes compressed media from magic bytes, never from the path
extension:

- PNG: the complete eight-byte PNG signature.
- JPEG: the leading JPEG start-of-image marker.
- GIF: `GIF87a` or `GIF89a`.
- WebP: a `RIFF` container with `WEBP` form type.

No raster decoder runs locally. A malformed or unsupported signature is
`image_unavailable` and is rejected after at most the fixed 12-byte signature
probe rather than reading the rest of that file or reserving its advertised
size. A supported descriptor receives one fallible exact-size reservation only
after admission. The fixed-size read scratch is allocated fallibly and lazily
only when bytes remain after the probe, then reused across images in the same
call. Animated GIF/WebP content is admitted as compressed input within the same
byte limits; interpreting frames belongs to the provider.

Every readable path receives a call-local positive `image_id` equal to its
one-based source position. Local failures retain that position. Healthy images
are processed in source order in sequential batches of at most eight images
and at most 8 MiB aggregate raw bytes. The next descriptor is opened, sized,
and signature-probed first; if its known size would cross either boundary, the
current batch is sent before allocating or reading the next complete snapshot.
At most 20 images and 64 MiB of aggregate image bytes are read by one call,
including bytes consumed by later local failures. A file that grows at the
exact aggregate boundary may consume one additional separately buffered
overflow-witness byte; after the budget is exhausted, remaining paths become
local failures without content reads. Admitted raw bytes are therefore also
bounded by 64 MiB.

## Private Gateway worker

The production `AiGatewayVisionTransport` reuses the configured model and the
same injected `Arc<dyn AiGatewayTransport>` as the outer provider. It uses a
dedicated, one-shot raw-v4 codec; it does not extend `ContentBlock`, `Prompt`,
the general Gateway history encoder, or durable session schemas.

Each batch request contains fixed system guidance, the bounded focus as user
text, then one file part per image:

```json
{
  "type": "file",
  "mediaType": "image/png",
  "data": "<standard base64>"
}
```

The request advertises no tools, selects tool choice `none`, and requests a
strict JSON response named `fx_vision_evidence`. The response schema
requires exactly one record per submitted image and admits only:

```json
{
  "images": [
    {
      "image_id": 1,
      "status": "ok",
      "summary": "A login form shows an invalid-token error.",
      "visible_text": ["Invalid token"],
      "details": ["The submit button remains enabled."]
    }
  ]
}
```

or a provider-declared batch failure record:

```json
{
  "images": [
    {
      "image_id": 1,
      "status": "failed",
      "error": "vision_unavailable"
    }
  ]
}
```

The decoder accepts bounded text deltas followed by one valid finish and
terminal record. A matching `text-start` / `text-end` pair may surround those
deltas but is not required by the pinned delta-only sequence; when present, its
ordering and identity are strict. Tool calls, tool results, media/file output,
contradictory or duplicate finish state, nonterminal output after finish,
malformed UTF-8/JSON, an unauthorized or duplicate image ID, and unknown fields
fail closed. `[DONE]` is the transport ownership boundary: the worker publishes
the already validated result and drops the source without polling a later
chunk. Transport authentication, rate-limit, timeout, unavailable, protocol,
response-size, and cancellation errors remain fixed and
path/body/credential-free.

A structurally invalid successful provider response, including a cleanly
finished empty evidence string, receives exactly one semantic retry using the
same owned, already verified image bytes. Transport failure, cancellation,
timeout, authentication, rate limiting, and typed output/resource-limit failure
are not retried. Structured JSON-node, evidence string/list/count, aggregate
evidence, and response-size exhaustion retain that output-limit classification
rather than becoming semantic invalidity. A valid `finishReason: length` is
also an output-limit failure. Gateway invalid-request and protocol failures are
unavailable provider responses, not semantic-success retry exhaustion. Batches
remain sequential. There is no backend fallback, live-provider retry policy,
or hidden parallel request.

## Result

The tool returns a path-free object whose `images` array exactly matches input
order. A partial provider response that omits one or more requested records
becomes `missing_provider_record` for each omitted image. An entirely empty
response is structurally invalid and becomes `provider_response_invalid`
after the one semantic retry. Local admission failure becomes
`image_unavailable`; provider/transport failure becomes `vision_unavailable`;
and exhausted evidence/output capacity becomes `output_limit_exceeded`.

Each provider batch independently obeys its 20 KiB evidence ceiling. If the
combined legal batch evidence would exceed the 48 KiB complete tool-result
ceiling, successful records are replaced from the source-order suffix with
fixed `output_limit_exceeded` records until the ordered total result fits. An
individually valid earlier result is never discarded in favor of a later one.

Failed records use this stable form:

```json
{
  "image_id": 2,
  "status": "failed",
  "error": {
    "code": "image_unavailable",
    "message": "Vision could not safely load or verify this image.",
    "retryable": false,
    "suggestion": "Explain the local image failure; do not retry the same snapshot unchanged."
  }
}
```

Successful evidence requires a bounded `summary` that remains nonempty after
trimming only ASCII space, tab, carriage return, and line feed; other Unicode
whitespace is preserved as provider evidence, matching the pinned contract.
`visible_text` and `details` are ordered bounded string arrays. Provider records
are reordered to the requested IDs. Extra, duplicate, or unauthorized records
invalidate the batch. The tool succeeds when at least one image succeeds and
sets `ToolOutput.is_error` only when every requested image fails. It never
exposes a path, MIME diagnostic, image byte, base64 value, request body,
provider body, credential, or endpoint diagnostic.

## Resource and lifecycle limits

| Resource | Limit |
| --- | ---: |
| source entries | 20 |
| focus | 4,096 bytes |
| normalized path | 4,096 bytes / 256 components / 255 bytes each |
| one compressed image | 8 MiB plus one overflow witness |
| aggregate image bytes read / admitted | 64 MiB plus at most one growth witness / 64 MiB |
| image read chunk | 64 KiB |
| images in one provider batch | 8 |
| raw bytes in one provider batch | 8 MiB |
| serialized worker request | 12 MiB |
| captured provider evidence per attempt | 20 KiB |
| complete provider response / one record | 64 KiB |
| serialized tool result | 48 KiB |
| userspace/network deadline, starting before capacity wait | 60 seconds |
| default / hard active executions | 2 / 8 |

Provider result string counts, per-string bytes, aggregate retained evidence,
SSE record count, and decoded JSON nodes have independent public constants and
remain below the complete response/result ceilings. The JSON-node budget is
one aggregate allowance across SSE event envelopes and final structured
evidence within each semantic attempt. Exact equality with a limit is
admitted; the first byte or item beyond it is rejected or converted to the
documented per-image limit failure.

Capacity is acquired before allocating image buffers and is held until all
descriptors, raw images, request/response buffers, transport futures, and
results for the call are dropped. The absolute deadline starts before capacity
waiting. Cancellation wins same-poll races and is checked before each file or
network effect, between fixed-size reads and encoding steps, after provider
readiness, before semantic retry, and before publication. Filesystem lookup and
read use synchronous operating-system calls: a kernel or remote filesystem call
that blocks inside one poll cannot be preempted by the userspace timer, and
deadline or cancellation observation may therefore wait for that call to
return. The deadline bounds controllable capacity, userspace, and network
phases; it is not a hard wall-clock bound on an uninterruptible system call.

Tool construction synchronously validates the target and opens, validates, and
retains the workspace-root descriptor; it performs no file-content or network
effect and starts no background work. Execution and transport futures are inert
until polled and detach no task or thread. Dropping an execution future before
its first poll performs no execution effect. Once an in-progress synchronous
filesystem call returns control to the future, dropping the call closes owned
descriptors, cancels/drops the transport future and byte stream, releases
buffers and capacity, and publishes no partial result. Once a transport is
polled, cancellation cannot retract bytes already accepted by the remote peer.
The private worker drops its response stream before returning so a capacity-one
shared Gateway transport cannot deadlock the next outer model round.

`Debug`, `Display`, and error values for the tool, requests, images, transport,
responses, and failures contain only fixed categories, counts, media kind, and
byte lengths. They never reveal focus, path, evidence, request/response bytes,
or provider diagnostics.

## Platform and feature scope

Portable media/request/response/error/limit and injected transport/deadline
contracts are available without HTTP or Tokio and on WebAssembly. The concrete
`VisionTool` type is available on non-WebAssembly native targets, but its
descriptor-reading implementation works only on Linux with `openat2` support
and macOS with `O_NOFOLLOW_ANY`; reference-host composition has the same
Linux/macOS boundary. Other native targets return the fixed
unsupported-platform construction failure and do not claim a working image
reader. Construction is network-inert.

## Deliberately deferred

This slice does not add durable image attachments, prompt images, CLI
`--image`, absolute or home-relative image paths, remote/data URLs, image
history, image generation, local OCR, raster decoding/resizing, artifact
persistence, progress events, live-provider tests, a dedicated provider model,
parallel batches, cache behavior, measured performance claims, or complete fx
equivalence. Those additions require separately reviewed authority, storage,
resource, and compatibility contracts.
