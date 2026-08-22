# Milestone 03 native reference-host composition review 01

Status: **CANDIDATE DOCUMENTATION ONLY — implementation, tests, composition,
adversarial review, exact remote gates, and `main` delivery pending**

## Candidate lineage

- Base and exact config-v2 delivery record:
  `a10f24edde80a225f89e6c7068ec035cb70f80a8`
- Exact config-v2 `main` CI run:
  [`32576876769`](https://github.com/distributedstatemachine/machine-god/actions/runs/32576876769)
- Exact config-v2 `main` benchmark-evidence run:
  [`32576876780`](https://github.com/distributedstatemachine/machine-god/actions/runs/32576876780)
- Candidate normative documentation: this branch commit; exact SHA reported at
  handoff
- Isolated production implementation: `PENDING`
- Isolated independent black-box tests: `PENDING`
- Composed candidate: `PENDING`
- Adversarially green behavior: `PENDING`
- Review record and exact feature-gate SHA: `PENDING`
- Documentation seal: `PENDING`
- Exact `main` delivery SHA: `PENDING`
- Integration branch: `agent/m03-native-reference-host`
- Candidate-docs branch: `agent/m03-native-reference-host-docs`
- Toolchain gate: Rust and Cargo 1.94.1 exactly

The base contains eleven integrated bounded Milestone 03 slices. This document
records only the proposed contract for the twelfth bounded library slice. It
does not assert that production code or tests are present on this branch, that
the candidate has been composed, that any reviewer is green, or that any exact
feature or `main` workflow has run. Milestone 03 remains `IN PROGRESS`.

## Proposed behavior awaiting composition and review

The candidate contract requires:

- a Linux/macOS-only API behind `ai-gateway-http` and the non-WebAssembly gate;
- `NativeReferenceHost::{compose_ai_gateway_http,
  compose_with_ai_gateway_transport, engine, into_engine, loaded_config,
  credential_source}` as the complete initial public host surface;
- validation of an already loaded `ask` / `vercel_ai_gateway` /
  `ai_gateway_http` selection before any other construction stage;
- one workspace open whose shared retained directory identity feeds exactly
  `list_files` and `read_file`;
- the existing `FileSessionStore` opened over a separate existing session root;
- `AskPermissionHandler` over an explicitly injected shared
  `PermissionPrompter`;
- `AiGatewayProvider` over either production `AiGatewayHttpTransport` built
  from the consumed injected credential snapshot or an explicit trusted custom
  `AiGatewayTransport` authority override;
- default `EngineLimits`, the default `NoopEventSink`, and no registered tool
  other than `list_files` and `read_file`;
- synchronous constructors that make no network request, poll no prompt, touch
  no session record, create no root or runtime, and start no background work;
- production construction that opens non-secret workspace and session roots
  before credential discovery and bearer-token handoff;
- a host-owned, driven Tokio runtime only when the production HTTP transport is
  later polled;
- exact retention of the supplied `LoadedNativeConfig`, including file-backed
  schema-v1 origin/version observation together with its fixed projected
  provider, transport, and model values;
- `Some(AiGatewayCredentialSource)` after production discovery and `None` for
  the custom-transport path, with no secret getter and no implication that a
  custom transport is unauthenticated;
- fixed `NativeReferenceHost { .. }` debug output; and
- a redacted `NativeReferenceHostBuildError` with a non-exhaustive stage kind:
  `UnsupportedSelection`, `WorkspaceRoot`, `SessionStore`, `Credential`,
  `HttpTransport`, `Provider`, or `Engine`.

The complete proposed normative behavior, order, failure strings, trust
boundaries, and deferred scope are in
[`native-reference-host.md`](../native-reference-host.md).

## Parallel delivery state

Production implementation and independent black-box tests are assigned to
separate isolated worktrees with non-overlapping ownership. This candidate
documentation must be reconciled with their exact composed API and behavior
before review. Three fresh adversarial tracks must then review one exact
composed commit for correctness/API, security/abuse, and
performance/concurrency/documentation scope. Confirmed findings must be fixed
and rereviewed until all three tracks are green.

## Candidate documentation checks

The following passed in the isolated candidate-docs worktree with exact
`rustc 1.94.1` and `cargo 1.94.1`:

- `cargo +1.94.1 test --doc --workspace`: two passed, zero failed;
- repository-relative Markdown links: 144 checked across 44 Markdown files,
  with none missing; and
- `git diff --check`.

Pre-commit worktree status contained only the intended candidate documentation
changes. These checks validate the candidate documents only; they are not
implementation, adversarial-review, exact remote, or delivery evidence.

## Remaining gates and scope

Implementation, independent tests, composed local required checks, three fresh
adversarial reviews, exact feature-branch CI and benchmark-evidence runs, the
documentation seal, fast-forward integration, and exact `main` CI and
benchmark-evidence runs all remain pending.

Root selection and safe creation, a terminal prompter, session identifiers and
lifecycle commands, the remaining native tools, CLI composition/execution, and
deterministic release-binary end-to-end evidence remain open. No compatibility,
full fx-equivalence, or product-performance claim is made. Zig remains solely
the pinned upstream benchmark build input; machine-god remains a Rust product.

The frozen reference-host composition checklist item remains unchecked while
this is documentation-only. It may be checked only after full delivery. The
combined credential-and-configuration item must remain unchecked after this
slice because config v2 has no bounded credential-source field. Milestone 03
remains in progress. No package or GitHub release is authorized.
