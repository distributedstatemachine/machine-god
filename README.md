# machine-god

`machine-god` is an experimental, embeddable coding-agent engine written in Rust.

The engine is the primary product. The command-line application is its native
reference host. Development status, architecture, compatibility, security, and
performance evidence live in [`docs/`](docs/README.md).

Milestones 01 and 02 are complete, and Milestone 03 is in progress with sixteen
delivered bounded slices plus a seventeenth candidate. The
repository includes the provider-neutral streaming engine, its bounded durable
tool loop, a deterministic testkit, read-only native configuration/status
discovery and loading, and capability-aware tool preflight before permission
policy. It also includes bounded Unix-only `read_file` and one-level
`list_files` library capabilities rooted in host-injected workspaces. The
seventeenth candidate adds bounded no-follow `file_info` metadata inspection
under a distinct exact authorization kind. It remains under local, adversarial,
and remote review and is not delivered. The repository also includes a bounded
AI Gateway codec over an injected host transport. An optional,
native-only HTTPS transport for that codec is the seventh integrated bounded
slice. It uses one pinned production endpoint and an explicitly injected,
redacted bearer token.
An eighth bounded slice implements a Unix file-backed session store under an
explicit host-opened root. Its exact feature, documentation-seal, and `main`
checks are green; it is integrated at
`8f7b47db9580b14570bf9fb55763858f71a81271`. Provider/CLI wiring, a concrete
prompt UI, remaining session-lifecycle delivery and hardening, the remaining
native tools, and compatibility work remain planned. A ninth bounded slice
defines an executor-neutral, fail-closed native `AskPermissionHandler` over an
explicitly injected prompter. It is integrated on `main` at
`27e3f2b3ff170044732d9124ffb210beabcda206`; exact main CI run `32570197911`
and benchmark run `32570197870` are green. It has no CLI or terminal authority.
See its [contract](docs/ask-permission.md).
The tenth integrated bounded slice adds opt-in native discovery of a validated
AI Gateway bearer credential from an owned, redacted environment snapshot. It
is integrated on `main` at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`; exact main CI run `32573320962`
and benchmark run `32573320937` are green. It does not change configuration
or CLI behavior; see its
[contract](docs/ai-gateway-credentials.md).
The eleventh integrated bounded slice advances the built-in native
configuration and current file schema to strict v2 with fixed declarative AI
Gateway provider, HTTP transport, and `zai/glm-5.2` model defaults. The exact
strict two-field v1 file remains read-compatible without rewrite or migration
and remains observable as schema v1 after its in-memory projection. Credentials
are in neither schema, and the new fields do not compose a provider, HTTP client,
runtime, token, network request, or CLI path. The feature implementation,
black-box tests, documentation, three adversarial tracks, and exact feature and
`main` gates are green. It is integrated on `main` at
`a10f24edde80a225f89e6c7068ec035cb70f80a8`; exact main CI run `32576876769`
and benchmark-evidence run `32576876780` are green. See the
[native configuration contract](docs/configuration.md) and
[review record](docs/reviews/m03-native-host-config-review-01.md).

A twelfth bounded library slice implements `NativeReferenceHost` for Linux
and macOS behind the optional `ai-gateway-http` and non-WebAssembly gate. It
composes an already validated native configuration with the AI Gateway
provider, either production HTTP from an injected credential snapshot or a
trusted injected custom transport, one shared retained workspace feeding
the delivered `list_files` and `read_file` tools, with candidate `file_info`
joining that same retained identity, the existing file session store, and the
ask handler over an injected prompter. Construction remains synchronous and
creates no root, runtime, network request, prompt operation, session record, or
background work. Its final delivery record is integrated on `main` at
`ac3984fb16dbab3adf86a949c7555ceca7c3e8df`; exact feature CI run
`32579779134`, feature benchmark-evidence run `32579779123`, main CI run
`32580066474`, and main benchmark-evidence run `32580066485` are green. The CLI
is byte-unchanged and remains thin. See the
[composition contract](docs/native-reference-host.md) and
[review record](docs/reviews/m03-native-reference-host-review-01.md).

A thirteenth bounded slice advances the strict current native
configuration to schema v3 by adding the required non-secret
`credential_source: "environment"` selection. Exact v1 and v2 files remain
strictly readable without rewrite and project the same acquisition kind only
in memory. The loader receives no token or process authority; the production
reference host still consumes an explicitly injected credential snapshot, and
its runtime observation still reports the concrete selected OIDC-token or
API-key source. Production implementation, independent tests, focused and
required local gates, and all three fresh adversarial tracks are green on exact
behavior SHA `35ce591e8ca6a8fef94485ff85d3e9c1397130a6`. It is integrated on
`main` through final delivery record
`f840576af241c58d1e55399e66ba92f7770cd50c`; exact final-record feature CI
run `32583585145`, feature benchmark-evidence run `32583585148`, main CI run
`32583871385`, and main benchmark-evidence run `32583871368` are green. See the
[configuration contract](docs/configuration.md) and
[review record](docs/reviews/m03-configured-credential-source-review-01.md).

The integrated fourteenth bounded slice adds explicit
Linux/macOS native root selection and safe preparation. It selects an existing
absolute workspace plus a state location from an injected native-environment
snapshot, opens and retains the workspace first, and may create only a fixed
descriptor-relative state suffix with private new-directory modes. It does not
change schema v3, configuration bytes, the CLI, status, or session lifecycle.
Production and 16 independently owned focused tests are present, and focused
root-selection and prepared-composition gates are green. Initial formal review
found fixture-mode and macOS ACL issues; those fixes and their ALLOW-rejection
and ordinary-HOME DENY-compatibility regressions are composed. All three formal
tracks were green together on exact behavior SHA
`f1dc47517d5b2d6d37628be4eb2ab51871e20b5d`. The first documentation seal then
exposed three Linux-only strict-Clippy diagnostics in feature CI run
`32588948956`; its native Linux/macOS tests and benchmark run `32588948975`
were green. Portable lint normalization is present at `90d8f96`, and local
macOS plus Linux cross-target gates are green. All three final adversarial
tracks are green on exact candidate SHA
`72cf64f63e0dfa30bc1ee21d8aca16550e819c21`. Replacement documentation seal
`f08dbd9eb2da81848b8eefb2d218006a64575835` is green under exact feature CI run
`32589778343` and benchmark-evidence run `32589778374`. Feature-evidence record
`6f66b6e5972e78ba0f0ccae06b899158d99bc864` is green under exact feature CI
`32590128235` and benchmark evidence `32590128233`; it is fast-forwarded without
force to `main` and green there under exact CI `32590429626` and benchmark
evidence `32590429592`. This documentation-only commit is the final delivery
record; its exact workflows are reported at handoff. See the integrated
[native root-selection contract](docs/native-root-selection.md) and
[review record](docs/reviews/m03-native-root-selection-review-01.md).

The delivered fifteenth slice implements a Linux/macOS native by-ID
session lifecycle over the exact `FileSessionStore` shared with the composed
engine. The caller supplies a validated session ID; the native host uses OS
randomness for each new incarnation. Create durably publishes an empty record,
resume returns the engine-canonical current lifetime, replay returns a bounded
durable `SessionRecord` snapshot rather than UI events, and reset atomically
publishes an empty new incarnation while advancing the durable revision. It
does not add session listing or any CLI behavior. Production, fourteen
independently owned focused tests, and one formal finding regression are green;
all three adversarial tracks are green together on exact candidate `e6a3804`.
Feature record `dbba2c7` is green under feature CI `32594562796`, feature
benchmark evidence `32594562785`, `main` CI `32594846484`, and `main` benchmark
evidence `32594846476`. This documentation-only commit is the final record; its
workflows are reported at handoff. See the integrated
[native session-lifecycle contract](docs/native-session-lifecycle.md).

The delivered sixteenth slice adds bounded Linux/macOS library-only session
listing. `NativeSessionLifecycle::list_sessions` returns at most 100 sorted,
unique validated IDs plus a truncation flag while processing at most 1,024
non-dot entries plus one fetched/name-inspected overflow witness and accepting/
decoding at most 64 MiB of aggregate canonical record bytes plus one transient
transfer byte used only to detect concurrent growth. Canonical corruption fails
closed; unrelated names are ignored but count against the scan budget.
The result is neither a multi-record snapshot nor a pagination or summary
surface. Production, documentation, and 13 initial independent tests are
composed from base `9ada4b5` through first formal candidate `dec98e0`. All three
first review tracks were not green. Isolated fix `4b8d8b0` and test hardening
`446b495` are composed into exact behavior candidate `3fa5463` with the
corrected documentation; its 18 focused tests, required local gates, and all
three replacement review tracks are green. First remote CI run `32599591900`
exposed a Linux removed-root
liveness gap. Exact portable-fix candidate `17f1884` applies the descriptor
check and is green under both executable review tracks. Documentation seal
`d3312d7` resolves the lineage finding, passed exact feature CI `32600292770`
and benchmark evidence `32600292779`, was fast-forwarded without force to
`main`, and passed exact main CI `32600567094` and benchmark evidence
`32600567090`. It adds no
CLI behavior and makes no fx equivalence or performance claim. See the
[native session-listing contract](docs/native-session-listing.md) and
[review record](docs/reviews/m03-native-session-listing-review-01.md).

The seventeenth bounded candidate adds Linux/macOS library-only `file_info`.
Strict effect-free preflight accepts only a required 4,096-byte-bounded
workspace-relative path, prepares `FilesystemAccess::Metadata`, and gives
policy and execution the exact same normalized path. Allowed execution walks
ancestors descriptor-relatively without following symlinks, then inspects the
final component with no-follow metadata without opening it. The exact bounded
result reports normalized path, fixed kind, checked size, signed Unix modified
time, and a nullable lexical regular-file extension. Final symlinks report
themselves; FIFO, socket, device, and other special objects are classified
without being opened. Reference-host composition grows from two to exactly
three workspace tools: `file_info`, `list_files`, and `read_file`. Core exposes
that catalog in deterministic alphabetical order. Production is present at
isolated SHA `5c2d129`; independent tests are present at isolated SHA `ca0091c`
and compose with production at `f228c06`, where all 34 focused tests are green.
Three fresh adversarial tracks, required full local gates, and exact feature and
`main` workflows remain pending, so the overall native-tool checklist stays open. See the
[`file_info` candidate contract](docs/file-info.md) and
[review record](docs/reviews/m03-file-info-review-01.md).

The project is not yet production-ready. See the exact
[CLI contract](docs/cli.md),
[`read_file` contract](docs/read-file.md),
[`list_files` contract](docs/list-files.md),
[`file_info` candidate contract](docs/file-info.md), and
[AI Gateway codec](docs/ai-gateway.md) plus
[native HTTP transport](docs/ai-gateway-http.md) and
[credential discovery](docs/ai-gateway-credentials.md) contracts, and the
normative [native file session store](docs/session-store.md), the integrated
[native reference-host composition](docs/native-reference-host.md), and the
integrated [configured credential source](docs/configuration.md), and the
integrated [native root-selection boundary](docs/native-root-selection.md).
The [native session lifecycle](docs/native-session-lifecycle.md) is integrated;
its bounded [session-listing extension](docs/native-session-listing.md) is a
delivered and green under exact feature and `main` workflow evidence.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

The Rust project is licensed under Apache-2.0. It is inspired by
[`vercel-labs/fx`](https://github.com/vercel-labs/fx), whose pinned comparison
revision is recorded in `benchmarks/upstream.lock`. Zig is pinned only to build
that upstream benchmark reference; it is not a machine-god product language or
runtime dependency.
