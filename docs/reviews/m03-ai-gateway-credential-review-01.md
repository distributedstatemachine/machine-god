# Milestone 03 native AI Gateway credential discovery review 01

Status: **DELIVERY GREEN — integrated on `main`**

## Reviewed lineage

- Base: `722b55a901a1a07b75a7097918464ad76ac79309`
- Production integration: `0724f595b494368ea30186bdc2e1f8db10e9c4db`
- Candidate documentation: `175d05ebff84ed8d7a4685a7354293953d523ea3`
- Initial HTTP bridge: `7409bdfa5af7f106fcafa773fb97416e2d078d41`
- Independent black-box tests: `8085ae693fd23cb339c5d54cd52ffb06b67993f1`
- Ambient-secret test and stale-status remediation:
  `06e37ca679f8181ba8025128e310c784c7e5cb84`
- Final API-key fallback and portability coverage:
  `244e765713944b1bbe2ebca5bbbd02899c725e9f`
- Branch: `agent/m03-ai-gateway-credential`
- Toolchain: Rust and Cargo 1.94.1 exactly

The slice adds an opt-in native credential-discovery adapter behind the
existing `ai-gateway-http` and non-WASM gate. Core, strict configuration schema
v1, the CLI, the provider codec, and the HTTP transport remain unchanged.

## Reviewed behavior

- `AiGatewayCredentialEnvironment::new` owns and immediately classifies only
  explicitly injected `VERCEL_OIDC_TOKEN` and `AI_GATEWAY_API_KEY` values.
  `from_process` and the process convenience function are the only ambient
  lookup paths.
- A nonempty OIDC token has fixed precedence over a nonempty API key. Exactly
  empty values are absent and fall through. A selected nonempty invalid value
  fails closed without falling back.
- Non-Unicode selected input maps to a fixed invalid-environment error.
  Unicode input reuses the existing 1–4,096-byte RFC 6750 `b64token` validator;
  syntax and size failures share one fixed invalid-bearer category.
- Discovery consumes the snapshot. The selected token moves into a
  non-cloneable discovered result and then into the HTTP transport; an unused
  valid lower-priority token is dropped.
- Snapshot, result, bearer-token, and error formatting do not reflect secret
  bytes. Errors retain no environment value, source, validator diagnostic, or
  operating-system detail.
- Accepted and retained credentials are bounded. Process lookup may first
  materialize the complete operating-system value. Clearing owned buffers is
  best effort, not locked memory or comprehensive zeroization.
- The credential-to-HTTP bridge proves both OIDC selection and empty-OIDC API
  key fallback produce exactly one expected `Authorization` header without
  forwarding the unselected synthetic token.
- The slice performs no persistence, configuration mutation, root creation,
  network request, CLI action, credential rotation, or provider composition by
  itself.

## Parallel implementation

Production, black-box test design and implementation, candidate documentation,
and the HTTP handoff test were developed in isolated worktrees with
non-overlapping ownership, then composed on the integration branch. Three
fresh adversarial tracks reviewed security/lifetime, API/testing/portability,
and documentation/plan/evidence.

## Adversarial rounds

### Round 1 — `8085ae693fd23cb339c5d54cd52ffb06b67993f1`

The security/lifetime track reported **GREEN**. The other tracks found four
accepted issues:

- **MEDIUM:** subprocess tests copied real parent-process credential values and
  could reflect them through `assert_eq!` diagnostics on failure. The
  unnecessary parent reads and assertions were removed; all environment
  injection remains confined to child `Command` instances.
- **LOW:** the HTTP bridge proved only OIDC handoff. A second bridge now proves
  an exactly empty OIDC value falls through and moves the API-key bytes into the
  sole authorization header.
- **LOW:** selected non-Unicode API-key behavior reached a separate match arm
  but lacked direct coverage. Unix and Windows fixtures now cover and redact
  that path.
- **MEDIUM:** `docs/core-api.md` still called the already integrated ninth ask
  handler a candidate. It now records the integrated status and links its
  contract and delivery evidence.

No production behavior or public API change was required. No finding was
rejected.

### Final round — `244e765713944b1bbe2ebca5bbbd02899c725e9f`

All three tracks reported **GREEN**. They confirmed both source-to-wire paths,
both selected non-Unicode branches, absence of real ambient secret capture in
tests, exact precedence and fail-closed behavior, platform/feature gates,
redacted diagnostics, bounded retained data, authority separation, and honest
candidate/evidence wording.

## Exact local checks

The following passed on
`244e765713944b1bbe2ebca5bbbd02899c725e9f` with exact Rust/Cargo 1.94.1:

- formatting;
- locked workspace/all-target/all-feature Clippy with warnings denied;
- locked default-feature workspace tests, all-feature workspace tests, and
  workspace documentation tests;
- all 13 credential tests: eight injected/concurrency cases, three isolated
  process cases, and two credential-to-wire HTTP cases;
- repo-wide Python discovery: 129 run, 121 passed and 8 expected platform
  skips;
- `cargo-deny` dependency policy, with only the accepted duplicate `syn` and
  `windows-sys` warnings;
- `cargo-audit`: 1,225 advisories checked across 174 lockfile dependencies with
  no vulnerability finding;
- `x86_64-unknown-freebsd` no-default native-library Clippy with warnings
  denied;
- `wasm32-wasip1` no-default and all-feature compilation, with only the
  pre-existing unrelated `read_file` dead-code warning;
- `aarch64-apple-darwin` no-default compilation;
- exact release CLI build plus bare, help, version, and JSON-status smoke; and
- `git diff --check` and a clean worktree.

The exploratory FreeBSD all-feature build requires a FreeBSD C sysroot for the
optional AWS-LC dependency and is not an applicable local gate. Native Linux
and macOS all-feature coverage remains owned by exact remote CI.

## Exact feature-branch delivery gates

The reviewed behavior and its documentation record are feature-green at
`11b661b927365ab207f1a1e8157e50a63fd07be4`:

- exact feature CI run
  [`32572692175`](https://github.com/distributedstatemachine/machine-god/actions/runs/32572692175)
  is green; and
- exact feature benchmark-evidence run
  [`32572692217`](https://github.com/distributedstatemachine/machine-god/actions/runs/32572692217)
  is green.

The documentation-only seal was required to pass both exact feature workflows
before it could be fast-forwarded without force to `main`; the exact `main`
SHA was then required to pass both workflows before delivery could be recorded.

The documentation seal at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0` passed exact feature CI run
[`32573044224`](https://github.com/distributedstatemachine/machine-god/actions/runs/32573044224)
and benchmark-evidence run
[`32573044159`](https://github.com/distributedstatemachine/machine-god/actions/runs/32573044159).

## Exact `main` delivery gates

The seal was fast-forwarded without force to `main` at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`. Exact main CI run
[`32573320962`](https://github.com/distributedstatemachine/machine-god/actions/runs/32573320962)
and benchmark-evidence run
[`32573320937`](https://github.com/distributedstatemachine/machine-god/actions/runs/32573320937)
are green for that exact SHA.

## Scope

This slice does not change configuration schema v1, add stored credentials,
select a model or endpoint, compose a provider, create a Tokio runtime, add a
prompt UI, expand the CLI, or complete Milestone 03. Benchmark-workflow success
validates the evidence path only. Zig remains solely a build input for the
pinned upstream fx comparison; machine-god remains a Rust product. No package
or GitHub release is authorized.
