# Milestone 03 native reference-host composition review 01

Status: **ADVERSARIALLY GREEN — implementation, independent tests, and local
gates green; exact remote feature gates and `main` delivery pending**

## Candidate lineage

- Base and exact config-v2 delivery record:
  `a10f24edde80a225f89e6c7068ec035cb70f80a8`
- Exact config-v2 `main` CI run:
  [`32576876769`](https://github.com/distributedstatemachine/machine-god/actions/runs/32576876769)
- Exact config-v2 `main` benchmark-evidence run:
  [`32576876780`](https://github.com/distributedstatemachine/machine-god/actions/runs/32576876780)
- Isolated candidate documentation:
  `e03f0fe0e0759566f062b7987b5e9d51497c0b43`
- Isolated production implementation:
  `dd062636b38783ef4d7dc987e9b10a8a0e19c903`
- Isolated independent black-box tests:
  `b24741c6e9e3ccb8744c2f638735c03b949c8e68`
- Initial composed candidate:
  `c261e84471f0a94cc644be92a6053e16af3ff6d7`
- Candidate state and security reconciliation:
  `25089e9765b313b27742da3f4342b90647d0a1af`
- Adversarially green behavior:
  `5afda631b83ee0ebd65ddc0e1d49079739b4914d`
- Review record and exact feature-gate SHA: this branch commit; exact SHA
  reported at handoff
- Documentation seal: `PENDING`
- Exact `main` delivery SHA: `PENDING`
- Integration branch: `agent/m03-native-reference-host`
- Candidate-docs branch: `agent/m03-native-reference-host-docs`
- Toolchain gate: Rust and Cargo 1.94.1 exactly

The base contains eleven integrated bounded Milestone 03 slices. Production,
independently owned tests, and candidate documentation for the twelfth bounded
library slice are composed on this branch. All three fresh adversarial tracks
are green on exact behavior SHA `5afda631`; no exact feature or `main` workflow
has run. Milestone 03 remains `IN PROGRESS`.

## Adversarially green behavior

The composed candidate implements:

- a Linux/macOS-only API behind `ai-gateway-http` and the non-WebAssembly gate;
- `NativeReferenceHost::{compose_ai_gateway_http,
  compose_with_ai_gateway_transport, engine, into_engine, loaded_config,
  credential_source}` as the complete initial public host surface;
- validation of an already loaded `ask` / `vercel_ai_gateway` /
  `ai_gateway_http` selection before any other construction stage;
- one workspace open whose shared retained directory identity feeds exactly
  `list_files` and `read_file`;
- the existing `FileSessionStore` opened over a separately supplied existing
  session root, with disjoint selection assigned to the trusted host because
  composition does not compare identity or ancestry;
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

The complete normative behavior, order, failure strings, trust
boundaries, and deferred scope are in
[`native-reference-host.md`](../native-reference-host.md).

## Parallel delivery and adversarial review

Production implementation, independent black-box tests, and candidate
documentation were completed in separate isolated worktrees with non-overlapping
ownership, then composed without conflicts. The seven tests exercise v1/v2
projection, exact model and two-tool registration, normalized permissions,
durable tool results, production credential precedence and inert construction,
redacted stage failures, custom-transport inertness, and shared retained
workspace identity after path replacement.

Three fresh adversarial tracks reviewed exact committed candidates for
correctness/API/tests/portability, security/resources/authority, and maintained
documentation/plan scope:

- On initial composed SHA `c261e844`, the API/test track was green. The security
  track found one low-severity documentation gap: root disjointness was implied
  but not enforced or assigned to the trusted host. The documentation track
  found one medium-severity state gap: maintained pages still described the
  already composed branch as documentation-only.
- SHA `25089e97` fixed both findings by reconciling all maintained status pages
  and explicitly documenting that composition does not compare root identity or
  ancestry, the trusted host must select disjoint roots, and overlap can expose
  session artifacts to workspace tools after permission. Security and API/test
  rereviews were green; the documentation rereview found one low-severity
  singular-verb typo.
- SHA `5afda631` fixed that typo. All three tracks independently reported
  **GREEN** on that exact final behavior SHA with no remaining finding.

No finding was rejected.

## Exact local checks

The following passed on adversarially green behavior SHA `5afda631` or its
code-identical documentation descendants with exact `rustc 1.94.1` and
`cargo 1.94.1`:

- `cargo +1.94.1 fmt --all -- --check`;
- `cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings`;
- `cargo +1.94.1 test -p machine-god-native --all-features --test reference_host`:
  seven passed, zero failed;
- `cargo +1.94.1 test --workspace`;
- `cargo +1.94.1 test --doc --workspace`: two passed, zero failed;
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
  smoke;
- repository-relative Markdown links: 144 checked across 44 Markdown files,
  with none missing; and
- `git diff --check` and a clean worktree.

These checks and the three green review tracks are local exact-SHA evidence;
they are not remote or `main` delivery evidence.

## Remaining gates and scope

Exact feature-branch CI and benchmark-evidence runs, the documentation seal,
fast-forward integration, and exact `main` CI and benchmark-evidence runs remain
pending.

Root selection and safe creation, a terminal prompter, session identifiers and
lifecycle commands, the remaining native tools, CLI composition/execution, and
deterministic release-binary end-to-end evidence remain open. No compatibility,
full fx-equivalence, or product-performance claim is made. Zig remains solely
the pinned upstream benchmark build input; machine-god remains a Rust product.

The frozen reference-host composition checklist item remains unchecked until
full delivery. The
combined credential-and-configuration item must remain unchecked after this
slice because config v2 has no bounded credential-source field. Milestone 03
remains in progress. No package or GitHub release is authorized.
