# Milestone 03 native host configuration schema-v2 review 01

Status: **CANDIDATE — adversarial review, exact remote gates, and `main`
delivery pending**

## Review lineage

- Base and tenth-slice credential delivery:
  `446859f8ca66b2310c12a8506f1771f4711b2361`
- Isolated production implementation:
  `1c4b01d4e63bde9562ceb62de2fe45a9112d2d6b`
- Composed production implementation:
  `99327a8127442f7ae12bb7acb31beb73b954bdb6`
- Isolated candidate documentation:
  `6c59ee58700c41a740c6abb89ea4483f80a40d32`
- Composed candidate documentation:
  `09c2221effa03497021e55d424c7531f57ad6bfb`
- Isolated independent black-box tests:
  `85ee96c95a7a59b6756ccdee0174b193c9c8bd94`
- Initial composed candidate:
  `4ae2ceb93d375630569fef5f6bc47b647051fa60`
- All-target Clippy test remediation:
  `f6f12fcb3288a2bcf45cd6049e19e9fd68bb1111`
- String-only schema remediation and current behavior candidate:
  `0d8e4590d76a6e58207951ff9d746c0c95cde003`
- Adversarially green candidate: `PENDING`
- Documentation seal: `PENDING`
- Exact `main` delivery SHA: `PENDING`
- Integration branch: `agent/m03-native-host-config-v2`
- Candidate-docs branch: `agent/m03-native-host-config-v2-docs`
- Toolchain gate: Rust and Cargo 1.94.1 exactly; results pending

This is the eleventh bounded Milestone 03 candidate. The first ten slices are
integrated. This slice's production implementation, tests, and documentation
are composed on its feature branch, while final adversarial rereview, exact
remote gates, and `main` delivery remain pending. Milestone 03 remains
`IN PROGRESS`.

## Reviewed behavior

The candidate presents the following behavior for adversarial review; no item
in this section is yet an adversarial or delivery-green claim:

- `CONFIG_SCHEMA_VERSION` advances to `2`, and a missing file or unavailable
  config location returns this strict built-in object:

  ```json
  {"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2"}
  ```

- A schema-v2 file must be one exact object with all five required fields.
  Unknown or duplicate fields, missing fields, wrong types or shapes,
  unsupported provider/transport/permission values, and invalid model values
  fail closed.
- The exact strict schema-v1 object
  `{"schema_version":1,"permission_mode":"ask"}` remains read-compatible.
  It maps only in memory to the fixed v2 provider, transport, and model values;
  its public schema version stays `1`, and loading never rewrites or migrates
  its file.
- `NativeProviderKind::VercelAiGateway` and
  `NativeTransportKind::AiGatewayHttp` expose the stable names
  `vercel_ai_gateway` and `ai_gateway_http`. `NativeConfig` exposes read-only
  `schema_version`, `permission_mode`, `provider`, `transport`, and `model`
  getters.
- `AI_GATEWAY_DEFAULT_MODEL` is `zai/glm-5.2`,
  `AI_GATEWAY_MAX_MODEL_BYTES` is `128`, and config models use the provider's
  same 1–128-byte visible-ASCII (`0x21..=0x7e`) validator.
- `NativeConfig` and `LoadedNativeConfig` are `Clone` but not `Copy` because
  the config owns its bounded model string. Config debug output redacts that
  model while retaining non-secret structural fields.
- Credentials appear in neither v1 nor v2. The separately integrated tenth
  credential-discovery slice retains its own secret and authority boundary.
- Provider, transport, and model values are declarative only. Parsing does not
  instantiate a provider, construct HTTP, select or drive an async runtime,
  discover or attach a token, perform network I/O, or compose the CLI.
- The transport enum and `ai_gateway_http` wire name exist even when the
  optional concrete HTTP transport is not compiled, including no-default and
  WebAssembly builds. A parsed selection therefore does not assert runtime
  transport availability.
- The existing 64 KiB raw-input cap, full-buffer UTF-8 precedence, Unix final-
  component no-follow/nonblocking open, authoritative descriptor regularity,
  at-most-one overflow witness byte, typed redacted failures, read-only
  behavior, and environment/path semantics remain unchanged.
- Native status remains metadata-only. It does not load this configuration,
  and all existing CLI bytes remain unchanged.

## Parallel implementation

Production, independent black-box tests, and candidate normative documentation
were developed in isolated worktrees with non-overlapping ownership and then
composed at the exact commits recorded above. Composition preserves the already
integrated credential slice and does not rewrite historical review records.

## Pending adversarial review and gates

Three fresh adversarial tracks remain required for:

- schema/API compatibility, exact v1/v2 dispatch, model-validator sharing,
  getters, trait surfaces, and tests;
- filesystem/resource/security behavior, strict duplicate handling,
  redaction, feature/target boundaries, and absence of credential or runtime
  authority; and
- documentation, plan/checklist honesty, unchanged CLI bytes, and evidence
  boundaries.

Every confirmed finding must be fixed and rereviewed until all three tracks are
green. Focused tests must precede the required exact-toolchain workspace gates.
The composed candidate and later documentation seal must pass exact feature-
branch CI and benchmark-evidence workflows; the exact fast-forwarded `main`
SHA must then pass both workflows. Candidate, adversarial, remote, and `main`
results are all pending at this record's current state.

## Scope

This candidate does not complete the combined credential-and-configuration
checklist item because it adds no bounded credential-source field and no
reference-host composition invokes credential discovery; final adversarial and
delivery evidence also remains pending. It adds no
provider/HTTP/runtime/token/CLI composition, workspace or state-root creation,
session lifecycle commands, remaining native tools, CLI expansion, deterministic
end-to-end composed-host evidence, compatibility promotion, or package/release
authorization. Those Milestone 03 items remain open.

No performance result or full fx-equivalence claim is made. A later green
benchmark-evidence workflow would validate only the retained evidence path for
that exact SHA. Zig remains solely the pinned upstream benchmark build input;
machine-god remains a Rust product.
