# Bounded MCP JSON Schema

`machine_god_native::mcp::schema` admits immutable JSON Schema data and evaluates
bounded JSON instances. It performs no reference I/O, executes no callbacks and
grants no permission. `McpSchema::parse` validates schema shape and every reached
reference before returning an assessment. Cloning shares the admitted immutable
schema, local resource indexes and bounded compiled-pattern cache.

The API separates hard errors from `LocallyEvaluable` and `ServerAuthoritative`
schema assessments. `validate_json` returns `Valid`, `Invalid(violation)` or
`ServerAuthoritative`; malformed JSON and exhausted budgets remain errors.
`require_object_root` independently enforces the exact MCP input-schema root
`"type":"object"`. Unions, booleans and reference-only object roots do not satisfy
that contract. Output schemas need not satisfy the input-root requirement.

## Lossless JSON and exact numbers

Schemas and instances are decoded through the established Serde JSON parser into
a bounded private arena. Numeric lexemes are retained as text, never converted
through an `f64`. Duplicate object keys, including escaped aliases, are rejected.
`raw_json` returns the original admitted schema spelling and whitespace. This
module does not enable Serde's global `arbitrary_precision` feature or silently
change provider/model projection behavior.

Number comparison normalizes a sign, significant decimal digits and a decimal
exponent; signed zero normalizes to zero. Integer checks, equality, enum/const
comparison and unique items preserve high precision and exponent spelling.
`multipleOf` checks decimal expansion bounds before calling the pinned pure-Rust
`num-bigint` remainder implementation. The native dependency is exactly 0.4.6,
with default features disabled and only `std` enabled. Its lock edges add
`num-integer` 0.1.47 and reuse `num-traits` 0.2.19; random generation and Serde
integration are not enabled. [Upstream package documentation](https://docs.rs/crate/num-bigint/0.4.6)
records its MIT/Apache-2.0 licensing and Rust 1.60 minimum.

## Dialects and local references

The default dialect is 2020-12. Only its exact HTTPS declaration and Draft 7's
exact HTTP declaration, with optional trailing `#`, are accepted. Other dialects
and conflicting nested declarations are hard errors. Other-dialect and unknown
keywords remain annotations; annotations become schema-checked if a reference
reaches them. Unknown vocabulary declarations are data, not new evaluator code.
Draft 7 `$ref` siblings are ignored, matching the pin.

The local resolver supports nested `$id` resources, relative RFC 3986 references,
JSON Pointer segments with percent and `~0`/`~1` decoding, ordinary anchors,
Draft 7 plain-name identifiers and 2020-12 dynamic anchors/references. Dynamic
lookup examines the actual active resource scope from outermost to innermost.
Duplicate resource identities/anchors, malformed targets, unresolved anchors and
unregistered external documents are hard errors. No URI performs filesystem or
network lookup. Referenced annotation objects carrying unindexed identifiers
are rejected; recursive annotation references may be admitted but evaluation
remains depth, step and reference-hop bounded.

Identifier spelling follows the producer's exact Zig 0.16 URI implementation,
including rendered `/` for an empty path, normalized numeric ports, its bounded
ASCII hostname checks and absolute-parse-to-relative fallback. It is not a URL
network parser: case and percent spellings remain intact, and permissively parsed
path text never gains filesystem or network authority.

The evaluator includes type/enum/const, exact numeric constraints, Unicode string
lengths, patterns, object/array counts, required and dependent fields, properties,
pattern/additional properties, property names, tuples/prefix items, items,
contains, unique items, dependencies, all/any/one-of, not and conditionals.
2020-12 evaluated-property/item annotations propagate only from successful
branches and govern `unevaluatedProperties`/`unevaluatedItems`. Formats and content
encodings remain annotations. Header-specific optional-null validation fallback
is not part of general JSON Schema and must remain in native MCP projection.

## Bounded pattern compatibility

Patterns use the pin's Thompson-machine subset: Unicode scalar literals,
concatenation, alternation, capturing groups, anchors, dot, character classes,
bounded/unbounded repetition and lazy-quantifier acceptance. ASCII digit/word
escapes, exact ECMAScript whitespace, literal/hex escapes and standalone Unicode
Letter property escapes follow the producer. Letter ranges are mechanically
transcribed Unicode 13.0 data, with original source hash and Unicode license
provenance beside the table. Matching does not use backtracking.

Grammar outside that evaluator, including lookaround/backreferences and forms
the pin cannot classify, produces a whole-schema `ServerAuthoritative`
assessment. All schema hard errors are checked first, regardless of keyword
order. A delegated schema still requires well-formed, bounded instance JSON;
it does not selectively apply local sibling constraints. Pattern budget overflow
is a hard error, not delegation. The retained pattern cache has an aggregate
state ceiling; uncached supported patterns compile on demand without rejecting
otherwise admitted schemas.

## Limits and compatibility

Callers may lower but not enlarge the following hard defaults:

| Budget | Default maximum |
| --- | ---: |
| Schema / instance bytes | 256 KiB / 1 MiB |
| JSON/evaluation depth | 64 |
| JSON nodes / entries per container | 4,096 / 4,096 |
| Evaluation steps / active reference hops | 100,000 / 256 |
| Number lexeme bytes | 4,096 |
| Absolute explicit/normalized decimal exponent | 1,000,000 |
| Expanded numerator/denominator digits | 8,192 each |
| Pattern states / finite repeat count | 2,048 / 1,024 |
| Aggregate pattern matching steps | 200,000 |
| Retained reference index charge | 1 MiB |
| Retained pattern cache bytes / states | 256 KiB / 2,048 |
| Complete retained schema charge | 4 MiB |

`retained_byte_charge` reports a conservative allocation budget, not allocator
telemetry. It includes original JSON, arena vector capacities and decoded text,
sparse object/index map overhead, resolved URI strings, anchor names, and cached
pattern instruction/class/range capacities. Each map entry is charged as a
separate sparse node with sixteen key/value/link slots and fixed bookkeeping;
shared map nodes are deliberately overcounted. Immutable URI text is shared
between the resource table and URI index rather than copied twice. The synthetic
root URI is charged independently even when `max_schema_bytes` is only four.

Reference entries are checked against both aggregate reference and remaining
schema budgets before copying URI/anchor text into retained indexes. One URI
resolution is transient and bounded by the per-schema byte ceiling (its joined
path can temporarily combine two bounded inputs); inherited prefixes cannot
accumulate without aggregate charges. JSON parsing similarly has byte/node/depth
bounds before its measured arena charge is admitted. These temporary allocations
do not multiply across retained schemas or parallel detached workers.

Pattern cache charges include sparse map entries, source text, instructions,
classes and their range arrays, not just instruction count. Cache exhaustion
skips retention and preserves on-demand compilation. At most one bounded pattern
is compiled at a time; source scalars/ranges are bounded by schema bytes, syntax
nodes/classes and expanded instructions by the per-pattern state limit, and
compilation recursion by the existing grammar/depth bounds. A cache allocation
is charged before insertion. Reference or mandatory schema-storage exhaustion
is a hard atomic admission error; cache exhaustion neither rejects a supported
schema nor changes it to `ServerAuthoritative`.

Equality recursion is charged to evaluation work as well as normal validation.
Schemas and instances remain untrusted data after validation. Full native
executable publication must separately bind admitted schemas, exact arguments,
configuration/authentication and live session/turn authority; projection must
preserve exact numbers end to end.

Behavior follows fx `b1774fbf6c7602b503026f96f6e960e946c692ef`, particularly
`json_schema.zig`, `json_schema_resolver.zig`, `json_number.zig`,
`json_schema_pattern.zig` and its Unicode Letter table. Producer-derived tests
retain its representative JSON Schema Test Suite provenance. These contracts do
not claim complete MCP feature delivery or a performance benchmark result.
