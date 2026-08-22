# Milestone 03 native host configuration schema-v2 review 01

Status: **FEATURE REVIEW GREEN — exact remote gates and `main` delivery
pending**

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
- String-only schema remediation:
  `0d8e4590d76a6e58207951ff9d746c0c95cde003`
- Adversarially green candidate:
  `53645ce89997be82a28d98ea0bdf4d74ea4f0c4d`
- Documentation seal: `PENDING`
- Exact `main` delivery SHA: `PENDING`
- Integration branch: `agent/m03-native-host-config-v2`
- Candidate-docs branch: `agent/m03-native-host-config-v2-docs`
- Toolchain gate: Rust and Cargo 1.94.1 exactly; local results green

This is the eleventh bounded Milestone 03 candidate. The first ten slices are
integrated. This slice's production implementation, tests, and documentation
are composed on its feature branch, and three fresh adversarial tracks are
green. Exact remote gates and `main` delivery remain pending. Milestone 03
remains `IN PROGRESS`.

## Reviewed behavior

Three fresh adversarial tracks reviewed the following behavior at exact SHA
`53645ce89997be82a28d98ea0bdf4d74ea4f0c4d`:

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

## Adversarial rounds

### Round 1 — `4ae2ceb93d375630569fef5f6bc47b647051fa60`

The three tracks reported three accepted **MEDIUM** findings:

- Serde's externally tagged unit-enum representation accepted object forms
  such as `{"ask":null}` in addition to the contract's required string fields.
  Both wire schemas now deserialize these fields as strings, compare exact
  stable values, and test object rejection for v1 permission mode and every v2
  enum-like field.
- The required all-target/all-feature warnings-denied Clippy gate found five
  test-helper lints. The helpers and negative trait assertion were made
  Clippy-clean without changing production behavior.
- Candidate status and lineage incorrectly described already composed
  production code, tests, and documentation as pending. Every maintained page
  now distinguishes present feature-branch work from pending remote and
  `main` delivery evidence and records the exact isolated and composed commits.

No finding was rejected.

### Final round — `53645ce89997be82a28d98ea0bdf4d74ea4f0c4d`

All three tracks reported **GREEN**. They independently confirmed string-only
schema rejection, exact v1/v2 dispatch and values, model-validator parity,
trait and getter surfaces, redaction, bounded filesystem behavior, feature and
target separation, absence of credential/runtime/network/CLI authority, test
coverage, exact lineage, and honest plan and scope wording.

## Exact local checks

The following passed on
`53645ce89997be82a28d98ea0bdf4d74ea4f0c4d` with exact Rust/Cargo 1.94.1:

- formatting;
- locked workspace/all-target/all-feature Clippy with warnings denied;
- locked default-feature and all-feature workspace tests, plus workspace
  documentation tests;
- all 29 config black-box tests, 14 focused config module tests, 35 provider
  tests, and 11 CLI tests;
- repo-wide Python tests: 129 run, 121 passed and 8 expected platform skips;
- `cargo-deny` dependency policy, with only the accepted duplicate `syn` and
  `windows-sys` warnings;
- `cargo-audit` 0.22.2: 1,225 advisories checked across 174 dependencies with
  no vulnerability finding;
- `x86_64-unknown-freebsd` no-default native Clippy with warnings denied;
- `wasm32-wasip1` no-default and all-feature compilation, with only the
  pre-existing unrelated `read_file` dead-code warning;
- `aarch64-apple-darwin` no-default compilation;
- a fresh exact release CLI build plus bare, help, version, and JSON-status
  smoke; and
- `git diff --check` and a clean worktree.

## Pending delivery gates

This review-record commit and the later documentation seal must pass exact
feature-branch CI and benchmark-evidence workflows. The exact fast-forwarded
`main` SHA must then pass both workflows. Remote and `main` results are pending
at this record's current state.

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
