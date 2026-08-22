# Milestone 03 configured credential-source review 01

Status: **CANDIDATE DOCUMENTATION ONLY — implementation, independent tests,
composition, adversarial review, exact remote gates, and `main` delivery
pending**

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
- Candidate normative documentation: this branch commit; exact SHA reported at
  handoff
- Isolated production implementation: `PENDING`
- Isolated independent black-box tests: `PENDING`
- Initial composed candidate: `PENDING`
- Adversarially green behavior: `PENDING`
- Review record and exact feature-gate SHA: `PENDING`
- Documentation seal: `PENDING`
- Exact `main` delivery SHA: `PENDING`
- Integration branch: `agent/m03-configured-credential-source`
- Candidate-docs branch: `agent/m03-configured-credential-source-docs`
- Toolchain gate: Rust and Cargo 1.94.1 exactly

The base contains twelve integrated bounded Milestone 03 slices. This document
records only the proposed contract for the thirteenth bounded slice. It does
not assert that production code or independent tests are present on this
branch, that a candidate has been composed, that an adversarial reviewer is
green, or that any candidate feature or `main` workflow has run. Milestone 03
remains `IN PROGRESS`.

## Proposed behavior awaiting composition and review

The candidate contract requires:

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

The complete candidate configuration contract is in
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

The candidate adds no persistence or schema rewrite, auth-source compatibility
claim, CLI behavior, provider/network effect during loading, or product-
performance claim. Core receives no ambient credential or configuration
authority.

## Parallel delivery and pending adversarial review

Production implementation, independent black-box tests, and candidate
documentation are assigned to isolated worktrees with non-overlapping
ownership. Their exact commits and the composed SHA remain pending. The
composed candidate must reconcile symbols and behavior before three fresh
adversarial tracks review one exact commit for:

- schema/API compatibility, strict v1/v2/v3 dispatch, projections, public
  getters, target/feature surfaces, and independent tests;
- resource, authority, secret non-reflection, failure precedence, and
  reference-host acquisition/runtime-source separation; and
- maintained documentation, plan/checklist honesty, unchanged CLI and
  benchmark scope, and exact evidence boundaries.

Confirmed findings must be fixed and rereviewed until all three tracks are
green. Documentation-only seal and delivery-record commits do not require a
new adversarial review after the production feature is already green.

## Candidate documentation checks

The following passed in the isolated candidate-docs worktree with exact
`rustc 1.94.1` and `cargo 1.94.1`:

- `cargo +1.94.1 test --doc --workspace`: two passed, zero failed;
- repository-relative Markdown links: 154 checked across 46 Markdown files,
  with none missing; and
- `git diff --check` with only intended documentation changes.

Passing these checks validates only this candidate documentation. It is not
implementation, test, adversarial-review, exact remote, or delivery evidence.

## Remaining gates and scope

Implementation, independent tests, composed local required checks, three fresh
adversarial reviews, exact feature-branch CI and benchmark-evidence runs, the
documentation seal, fast-forward integration, and exact `main` CI and
benchmark-evidence runs all remain pending.

The combined credential-and-configuration checklist item remains unchecked
while those gates are pending; it may be checked only after full delivery. Root
selection and safe creation, session lifecycle behavior, the remaining native
tools, CLI expansion and composition, deterministic release-binary end-to-end
evidence, and compatibility promotion remain open. Milestone 03 remains in
progress. No package or GitHub release is authorized.

Zig remains solely the pinned upstream benchmark build input; machine-god
remains a Rust product.
