# MCP descriptor catalogs and candidates

`machine_god_native::mcp::catalog` admits complete descriptor families from
`McpRawCatalog` and builds immutable replacement candidates. Neither an admitted
descriptor nor a candidate grants permission, publishes a runtime generation,
connects to a server, or invokes a tool. The runtime retains ownership of
configuration/authentication, live turns, refresh, publication and cleanup.

## Descriptor admission

`McpDescriptorCatalog::admit` accepts one completed raw catalog family and checks
every descriptor before returning anything. It preserves the protocol version,
fetch/expiry observations and cache scope from raw assembly. Descriptors remain
sorted by their exact remote identities. Catalog clones share immutable storage.

All four pinned families are supported:

- Tools require a nonempty name and an admitted input schema with the exact
  root `"type":"object"`. Optional output schemas use general schema admission,
  including boolean schemas. Whole-schema `ServerAuthoritative` assessments
  remain valid admitted tools, not grounds for dropping a descriptor.
- Resources retain URI, name, title, description, MIME type, exact nonnegative
  `u64` size, icons, annotations and metadata. Integral decimal/exponent sizes
  and negative zero are supported without an `f64` conversion.
- Resource templates retain their exact URI template and shared fields. Their
  pinned grammar allows up to 64 nonadjacent single-variable expressions and
  the simple/reserved/fragment/label/path/path-parameter/query operators.
  Unsupported variable lists, explode/prefix modifiers, invalid percent escapes
  and malformed expressions are rejected. Template size, when supplied, is
  validated and retained in raw JSON but not projected as a resource size.
- Prompts retain name/title/description, icons, metadata and up to 128 ordered
  unique arguments with name, optional description and boolean requiredness.
  Missing requiredness means false. Unknown prompt annotations remain raw data,
  not the resource/tool annotation contract.

Names are at most 256 UTF-8 bytes, titles/MIME types 4,096 bytes, descriptions
64 KiB and schemas 256 KiB each. Metadata, icon and annotation JSON fields are
bounded to 128 KiB each. Icon arrays and icon-size arrays have at most 16 entries;
themes are exactly `light` or `dark`. Resource/prompt icon sources are at most
64 KiB, while tool icon sources have the producer's 1 MiB field ceiling within
the enclosing metadata bound. Required strings are nonempty; optional strings
may be empty but may not be null or another JSON type.

Tool annotations validate title and the four boolean hints. Resource annotations
validate audience, exact numeric priority in `[0,1]` and optional last-modified
text. Resource/prompt metadata depth is at most 32; tool metadata remains within
the admitted wire depth. Unknown descriptor fields are preserved rather than
treated as executable instructions.

Every typed descriptor retains its complete original raw JSON. Extracted schemas
and optional raw JSON fields preserve their numeric lexemes and object keys,
including keys that resemble private Serde tokens. No untrusted descriptor is
converted through `json!`, `serde_json::to_value` or a rounded numeric tree.
The raw byte ceilings count retained whitespace/escape spellings as well as
content; schema evaluation follows the [schema contract](mcp-schema.md).

## Deterministic candidate construction

`McpCatalogCandidate::build` consumes borrowed server inputs in configuration
order. It rejects duplicate server aliases, duplicate catalog families and
inconsistent protocol versions. Names inside each family retain the raw
assembler's byte-lexicographic order.

Exposed tool names are `mcp_<server>_<remote>`. Each non-ASCII or unsupported byte
becomes `_`; ASCII letters/digits, `_` and `-` remain unchanged. Names have at
most 64 ASCII bytes. Collisions append `_2`, `_3`, and so on while shortening the
base to stay within that bound. Reserved built-in names and names belonging to
other current catalogs participate in the same allocation. A refresh supplies
those other-current names, excluding its own replaced catalog's old names.
Repeated construction from identical ordered inputs/reservations is stable.

The modern-HTTP exposure policy requires exactly one explicit decision per tool,
with no duplicate or unknown remote identities. The owning runtime obtains those
decisions from the shared modern-HTTP input-header validator. This module does
not duplicate its parser. Only `ExcludeModernHttpHeaders` decisions omit tools
from exposure; exclusions retain their descriptor and reason, and do not reserve
names. All descriptors undergo complete schema/field admission before this step.
Standard stdio/legacy policies do not apply modern-HTTP exclusions.

Missing/empty tool descriptions project as `MCP tool`. Search tags use the
producer's ASCII tokenization/lowercasing, stable deduplication and 16-tag bound.
Search text includes full server/name/description/schema/tag text; schema text
uses the shared exact-JSON decoder and compact serializer so escaped keys and
strings are searchable by decoded spelling, without rounding numeric lexemes.
Original descriptor/schema accessors remain byte-preserving. Search text is private
catalog matching data, not an extra provider instruction.

`McpToolModelProjection` is an explicit borrowed full-fidelity name, server,
description, schema and tags view. It does not construct the older size-limited
`ToolSpec` or silently truncate/drop admitted descriptors. Model codec and
search/select adapters must explicitly support these full limits before runtime
activation; the projection itself grants no executable authority.

## Aggregate bounds and atomicity

Descriptor limits default to 2,048 tools or 4,096 items in each feature family,
64 servers, 131,072 reserved names, one million total name-allocation attempts,
and 64 MiB each for admitted-catalog/candidate retained byte charges. Callers may
lower, never enlarge, those limits. Raw pagination independently enforces its
page, item, response-byte and node budgets; descriptor admission cannot bypass
the raw stage.

Before copying descriptor data, admission charges four times its raw JSON byte
length for retained raw payloads and extracted text/JSON. Each input/output
schema additionally contributes its full `retained_byte_charge`, including
arena storage, aggregate reference indexes and compiled-pattern cache; raw
schema bytes are conservatively counted in both charges. Schema admission uses
the remaining catalog budget before retaining each descriptor. Parsing is
sequential, so transient storage is limited to one bounded descriptor and one
schema parse rather than a collection of uncharged admitted schemas.
Candidate construction charges shared catalog storage conservatively plus
server/reservation/eligibility text, full search text, tags and exposed names.
These are conservative retained allocation charges, not allocator heap telemetry;
fixed descriptor containers also have finite counts and schema internals have
independent aggregate byte/state/depth limits. Server and reserved-name counts/lengths are checked
before their copies. Name allocation has a shared work budget and remembers
already consumed suffixes without changing deterministic results.

Any malformed descriptor, schema, eligibility input or exhausted budget rejects
the entire construction. Existing candidates and caller reservations are never
mutated, so a failed reload can retain the previous usable runtime. Publication,
generation binding and retirement are separate native runtime operations.
Debug/error displays contain categories, not remote payloads or identities.

Behavior follows fx `b1774fbf6c7602b503026f96f6e960e946c692ef`, particularly
`mcp_runtime.zig` and `features/tools.zig`, `resources.zig`, `prompts.zig` and
`common.zig`. These component contracts do not claim complete MCP delivery or
performance benchmark acceptance.
