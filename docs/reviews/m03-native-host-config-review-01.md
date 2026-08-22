# Milestone 03 native host configuration schema-v2 review 01

Status: **CANDIDATE — adversarial review, exact remote gates, and `main`
delivery pending**

## Review lineage

- Base and tenth-slice credential delivery:
  `446859f8ca66b2310c12a8506f1771f4711b2361`
- Production implementation: `PENDING`
- Independent black-box tests: `PENDING`
- Candidate documentation: `PENDING`
- Composed candidate: `PENDING`
- Adversarially green candidate: `PENDING`
- Documentation seal: `PENDING`
- Exact `main` delivery SHA: `PENDING`
- Integration branch: `agent/m03-native-host-config-v2`
- Candidate-docs branch: `agent/m03-native-host-config-v2-docs`
- Toolchain gate: Rust and Cargo 1.94.1 exactly; results pending

This is the eleventh bounded Milestone 03 candidate. The first ten slices are
integrated; this candidate is not. Milestone 03 remains `IN PROGRESS`.

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
are assigned to isolated worktrees with non-overlapping ownership. Their
commits and the eventual composed SHA remain pending and will be recorded in
the lineage above. Composition must preserve the already integrated credential
slice and must not rewrite historical review records.

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
checklist item while integration and evidence remain pending. It adds no
provider/HTTP/runtime/token/CLI composition, workspace or state-root creation,
session lifecycle commands, remaining native tools, CLI expansion, deterministic
end-to-end composed-host evidence, compatibility promotion, or package/release
authorization. Those Milestone 03 items remain open.

No performance result or full fx-equivalence claim is made. A later green
benchmark-evidence workflow would validate only the retained evidence path for
that exact SHA. Zig remains solely the pinned upstream benchmark build input;
machine-god remains a Rust product.
