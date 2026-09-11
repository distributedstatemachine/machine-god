# Native MCP tool result admission

`NativeMcpToolResultAdmission` admits a complete, correlated `tools/call`
response as data. It performs no transport, permission, browser, archive or
continuation effects. The runtime supplies the exact selected descriptor,
protocol, request ID, exposed name, server and native runtime allocation in a
non-clone `McpToolResponseContext`. Capturing those values does not grant authority;
the execution owner must retain and revalidate its actual turn and route.

The complete duplicate-free JSON-RPC envelope is bounded before raw-field
admission. Null/foreign IDs, request envelopes, mixed result/error fields,
malformed content and unknown result types are rejected. Missing `resultType`
remains a complete result under both modern and legacy protocols, as at the pin.

## Distinct outcomes

- `Complete` retains the entire original result as exact JSON values, including
  structured content, unknown metadata, large numeric lexemes and signed zero.
  The server's boolean `isError` denotes a completed tool failure. If the admitted
  descriptor has an output schema, `structuredContent` is required and validated
  even for `isError: true`; server-authoritative schemas stay server-authoritative.
- `ProtocolFailure` retains its integer code, bounded message and original error
  object. It is not a completed tool failure. Errors and debug output never echo
  the message, configuration, authentication identity or response payload.
- `InputRequired` owns typed validated requests/state together with exact
  response provenance. Modern input-required data never becomes successful tool
  output. The pinned legacy 2025-11 URL-required error can enter the same custody;
  malformed legacy extension data remains a protocol failure.

Input custody is not cloneable or serializable. Its read-only getters and
consuming `into_parts` expose data, never a write grant. Native continuation
must separately retain original invocation arguments/options, obtain explicit
consent, validate responses and obtain a fresh exact permission-bound submission.
Neither a request ID nor serialized server state can restore an old grant.

## Shared content with method-specific policy

Tool and feature responses reuse one content implementation for text, image,
audio, resource links and embedded text/blob resources, including strict base64
and outer annotations/metadata. The pinned policies remain distinct:

| Field | Tool result | Resource/prompt feature |
| --- | --- | --- |
| URI and icon source limit | 1 MiB content-field limit | 64 KiB |
| Required MIME, URI, link name and icon source | Empty strings accepted | Empty strings rejected |
| Embedded resource annotations | Retained unknown data | Validated annotations |
| Aggregate content | Complete tool-result byte budget | Separate 4 MiB content budget |

Both policies still require the appropriate fields and exactly one embedded
text/blob representation. Accepting protocol strings does not authorize opening
their URI, interpreting content as instructions or writing it to a terminal.

## Bounds and projection ownership

Defaults permit 256 content items, 1 MiB content fields, a 4 MiB plus 16 KiB
compact complete result, a 16 MiB raw envelope, 262,144 JSON values/keys and depth
32 with the root at zero. Limits may only be lowered. Response admission charges
four times original bytes against a separate 64 MiB retained-data ceiling before
parsing; parser nodes and transient content decoding have independent finite
bounds. Protocol messages allow 64 KiB and protocol data allows 128 KiB compact
JSON. Output-schema validation retains its own instance/work bounds.

Complete output is not an already durable archive or a model/terminal projection.
The native execution integration must archive and sanitize/project it under the
existing complete-output contract. This codec neither silently truncates data
nor claims it fits ordinary inline transcript limits.

Behavior is derived from fx `b1774fbf6c7602b503026f96f6e960e946c692ef`,
`src/core/mcp/features/tools.zig`, `mrtr.zig`, `elicitation.zig` and the shared
feature content codecs. Full runtime/CLI delivery remains governed solely by the
[implementation plan](implementation-plan.md).
