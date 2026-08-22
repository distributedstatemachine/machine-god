# Milestone 03 configured credential-source review 01

Status: **DELIVERY GREEN — integrated on `main`**

## Candidate lineage

- Base and exact native reference-host final delivery record:
  `ac3984fb16dbab3adf86a949c7555ceca7c3e8df`
- Base reference-host adversarially green behavior:
  `5afda631b83ee0ebd65ddc0e1d49079739b4914d`
- Base reference-host documentation seal and integrated behavior:
  `86627d7834e418d5e7b65f6b8497e4bddfa53395`
- Exact base feature CI run:
  [`32579779134`](https://github.com/distributedstatemachine/machine-god/actions/runs/32579779134)
- Exact base feature benchmark-evidence run:
  [`32579779123`](https://github.com/distributedstatemachine/machine-god/actions/runs/32579779123)
- Exact base `main` CI run:
  [`32580066474`](https://github.com/distributedstatemachine/machine-god/actions/runs/32580066474)
- Exact base `main` benchmark-evidence run:
  [`32580066485`](https://github.com/distributedstatemachine/machine-god/actions/runs/32580066485)
- Isolated candidate normative documentation:
  `699f09a24286a4dfa94882d2500ac2d0b3fd066f`
- Candidate documentation as composed:
  `a8467cc4e889b0778b7b95968714b8c2b08d6fc4`
- Isolated production implementation:
  `b4351151b241928f4a8f6f1c28283fd2de7dc8e4`
- Production implementation as composed:
  `abfa6e4a2adc2874da91e2b551c45f788d6241e8`
- Isolated independent black-box tests:
  `00f08d1da4bd0f3c3510bfb5af3e53715eb81647`
- Independent tests as composed:
  `187eaefb1995c8cc75ed3a573a2c41541e20f5f6`
- Initial composed and adversarial-review candidate:
  `a8467cc4e889b0778b7b95968714b8c2b08d6fc4`
- Documentation status reconciliation:
  `35ce591e8ca6a8fef94485ff85d3e9c1397130a6`
- Adversarially green behavior:
  `35ce591e8ca6a8fef94485ff85d3e9c1397130a6`
- Adversarial-green review record and documentation seal:
  `5f4deac672af85fe5c0b1be50c327ddbdd55ce9a`
- Exact feature-gate SHA:
  `5f4deac672af85fe5c0b1be50c327ddbdd55ce9a`
- Exact feature CI run:
  [`32582210892`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582210892)
- Exact feature benchmark-evidence run:
  [`32582210927`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582210927)
- Documentation-only feature evidence record:
  `8755757da0da07e33af48d57f46bd9ea490b5449`
- Exact feature-evidence-record CI run:
  [`32582687145`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582687145)
- Exact feature-evidence-record benchmark run:
  [`32582687169`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582687169)
- Exact `main` delivery SHA:
  `8755757da0da07e33af48d57f46bd9ea490b5449`
- Exact `main` CI run:
  [`32582978232`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582978232)
- Exact `main` benchmark-evidence run:
  [`32582978286`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582978286)
- Final delivery record:
  `f840576af241c58d1e55399e66ba92f7770cd50c`
- Exact final-record feature CI run:
  [`32583585145`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583585145)
- Exact final-record feature benchmark-evidence run:
  [`32583585148`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583585148)
- Exact final-record `main` CI run:
  [`32583871385`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583871385)
- Exact final-record `main` benchmark-evidence run:
  [`32583871368`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583871368)
- Integration branch: `agent/m03-configured-credential-source`
- Candidate-docs branch: `agent/m03-configured-credential-source-docs`
- Toolchain gate: Rust and Cargo 1.94.1 exactly

The thirteenth slice's production, independently owned black-box tests, and
maintained documentation are integrated on `main`. Focused checks and required
local workspace gates are green. All three fresh adversarial tracks are green on
exact behavior/finding-fix SHA
`35ce591e8ca6a8fef94485ff85d3e9c1397130a6`. The documentation seal and both
first exact feature workflows are green at `5f4deac6`. Feature-evidence record
`8755757d` passed both exact feature workflows, was fast-forwarded without force
to `main`, and passed both exact `main` workflows. Final documentation-only
record `f840576a` then passed exact feature and `main` CI and benchmark-evidence
workflows. Thirteen slices are integrated; Milestone 03 remains `IN PROGRESS`.

## Adversarially green behavior

The integrated slice implements:

- `CONFIG_SCHEMA_VERSION == 3` and this exact strict built-in object:

  ```json
  {"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}
  ```

- schema v3 to be the exact schema-v2 shape plus the sixth required string
  field `credential_source: "environment"`;
- `NativeCredentialSourceKind::Environment`, stable machine name
  `environment`, an `as_str` accessor, and the read-only
  `NativeConfig::credential_source` getter;
- exact strict v1 and v2 read compatibility without rewrite, expansion, or
  migration: each retains its loaded schema version and projects
  `NativeCredentialSourceKind::Environment` only in memory;
- schema v2 to reject `credential_source` as an unknown field, so the new wire
  selection requires explicit schema version `3`;
- strict rejection of missing, duplicate, unknown, wrong-type, or unsupported
  v3 fields and values under the unchanged 64 KiB plus one-witness-byte input
  bound and existing model validator;
- redacted fixed loader failures and config debug output that continues to
  redact the model while exposing only the non-secret credential-source enum;
- production `NativeReferenceHost` selection validation to require configured
  `Environment`, then use the already injected
  `AiGatewayCredentialEnvironment` under the existing discovery contract;
- `NativeReferenceHost::credential_source()` to retain its existing, distinct
  runtime meaning: the concrete selected `VERCEL_OIDC_TOKEN` or
  `AI_GATEWAY_API_KEY` source after production discovery; and
- the trusted custom-transport constructor to skip native discovery and keep
  returning `None` for that runtime observation.

The complete configuration contract is in
[`configuration.md`](../configuration.md). The existing credential and
composition boundaries remain in
[`ai-gateway-credentials.md`](../ai-gateway-credentials.md) and
[`native-reference-host.md`](../native-reference-host.md).

## Authority and redaction boundary

The configured `environment` value is a closed non-secret acquisition kind.
It carries no token bytes and no arbitrary environment-variable name. The
loader remains bounded, synchronous, and read-only and gains no process,
network, runtime, core, or CLI authority. Only an explicitly constructed
`AiGatewayCredentialEnvironment` snapshot reaches production composition; the
loader and reference-host constructors do not call `from_process`.

The slice adds no persistence or schema rewrite, auth-source compatibility
claim, CLI behavior, provider/network effect during loading, or product-
performance claim. Core receives no ambient credential or configuration
authority.

## Parallel delivery and adversarial review

Production implementation, independent black-box tests, and candidate
documentation were completed in isolated worktrees with non-overlapping
ownership, then composed at exact SHA
`a8467cc4e889b0778b7b95968714b8c2b08d6fc4`. Three fresh adversarial tracks
reviewed correctness/API/tests/portability, security/resources/authority, and
maintained documentation/evidence scope.

- Security and API/test/portability were **GREEN** on initial composed SHA
  `a8467cc4` and again on exact SHA `35ce591e`.
- The documentation track found one **MEDIUM** issue on `a8467cc4`: maintained
  pages still said implementation, independent tests, and composition were
  pending.
- Exact SHA `35ce591e` reconciled every maintained status page while preserving
  pending remote, seal, `main`, and checklist gates. The documentation track
  then reported **GREEN** on that exact SHA.

No finding was rejected. All three tracks are green together on exact behavior
SHA `35ce591e8ca6a8fef94485ff85d3e9c1397130a6`.

This documentation-only seal changes evidence and status text only. Per the
delivery workflow and the user's explicit instruction, it does not require a
new adversarial review after the production behavior is already green.

This feature-evidence record is likewise documentation-only and is not
adversarially reviewed under that same workflow and user instruction.

This final delivery record is also documentation-only and is not adversarially
reviewed under that same workflow and user instruction.

## Exact local checks

The following are green on `a8467cc4`, exact behavior descendant `35ce591e`,
or both where the latter changes documentation only, with exact Rust and Cargo
1.94.1:

- `cargo +1.94.1 fmt --all -- --check`;
- 29 focused native configuration tests and 15 all-feature configured-source
  tests;
- locked workspace all-target/all-feature Clippy with warnings denied;
- default-feature and all-feature workspace tests;
- workspace doc tests;
- repo-wide Python tests: 129 run, 121 passed, and 8 expected platform skips;
- `cargo-deny` 0.20.2 dependency policy;
- `cargo-audit` 0.22.2 vulnerability audit;
- API-review FreeBSD, WASI, and Apple target compile checks on `a8467cc4`,
  unchanged by the documentation-only `35ce591e` descendant, with only the
  pre-existing WASI `read_file` dead-code warning; and
- a fresh release binary whose bare, help, version, human-status, and JSON-status
  output matched the exact documented bytes while strict schema-v3 bytes and
  behavior remained unchanged; and
- candidate and status-reconciliation Markdown link and diff checks.

The documentation-only seal, feature-evidence descendant, and final delivery
record pass exact Rust/Cargo 1.94.1 workspace doc tests (two passed, zero
failed), 154 repository-relative links across 46 Markdown files with none
missing, and `git diff --check`. These local results are distinct from the exact
feature and `main` workflows below.

## Residual risk and deferred scope

The closed configured kind supports only `environment`; it is not an auth-
source compatibility claim. Exact v1/v2 files project that kind only in memory.
Process snapshot construction retains its existing limitation that the standard
library may materialize a complete OS value before bounded rejection. Runtime
`credential_source()` remains concrete OIDC-token/API-key metadata, while
`None` on the custom path does not prove a transport is secret-free.

## Exact feature-branch gates

Documentation seal `5f4deac672af85fe5c0b1be50c327ddbdd55ce9a`
passed exact feature CI run
[`32582210892`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582210892)
and exact feature benchmark-evidence run
[`32582210927`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582210927).
Both runs report that exact `headSha`. This is feature-delivery and retained-
evidence-path validation, not a product-performance claim.

Feature-evidence record `8755757da0da07e33af48d57f46bd9ea490b5449`
then passed exact feature CI run
[`32582687145`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582687145)
and exact feature benchmark-evidence run
[`32582687169`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582687169).
Both runs report that exact `headSha`.

Final delivery record `f840576af241c58d1e55399e66ba92f7770cd50c` then
passed exact feature CI run
[`32583585145`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583585145)
and exact feature benchmark-evidence run
[`32583585148`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583585148).
Both runs report that exact `headSha`.

## Exact `main` delivery

Feature-evidence record `8755757da0da07e33af48d57f46bd9ea490b5449` was
fast-forwarded without force to `main`. Exact main CI run
[`32582978232`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582978232)
and benchmark-evidence run
[`32582978286`](https://github.com/distributedstatemachine/machine-god/actions/runs/32582978286)
are green for that exact SHA.

Final delivery record `f840576af241c58d1e55399e66ba92f7770cd50c` was then
fast-forwarded without force to `main`. Exact main CI run
[`32583871385`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583871385)
and benchmark-evidence run
[`32583871368`](https://github.com/distributedstatemachine/machine-god/actions/runs/32583871368)
are green for that exact SHA.

## Remaining scope

The combined credential-and-configuration checklist item is complete. A
fourteenth documentation-only root-selection candidate is being developed
separately; root-selection delivery, session lifecycle behavior, the remaining
native tools, CLI expansion and composition, deterministic release-binary end-
to-end evidence, and compatibility promotion remain open. Milestone 03 remains
in progress. No package or GitHub release is authorized.

Zig remains solely the pinned upstream benchmark build input; machine-god
remains a Rust product.
