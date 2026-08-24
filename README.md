# machine-god

`machine-god` is an experimental, embeddable coding-agent engine written in Rust.

The engine is the primary product. The command-line application is its native
reference host. Development status, architecture, compatibility, security, and
performance evidence live in [`docs/`](docs/README.md).

Milestones 01 and 02 are complete, and Milestone 03 is in progress with
twenty-four delivered bounded slices. The twenty-fourth, library-only
`copy_file` slice is green under three fresh same-SHA adversarial reviews and
exact feature and `main` delivery gates. The twenty-fifth, library-only
`create_folder` contract is frozen from that delivered base; implementation,
evidence, reviews, host composition, and delivery remain pending. The
repository includes the provider-neutral streaming engine, its bounded durable
tool loop, a deterministic testkit, read-only native configuration/status
discovery and loading, and capability-aware tool preflight before permission
policy. It also includes bounded Unix-only `read_file` and one-level
`list_files` library capabilities rooted in host-injected workspaces. The
seventeenth delivered slice adds bounded no-follow `file_info` metadata
inspection under a distinct exact authorization kind. Its local gates, all
three replacement adversarial tracks, and exact feature and `main` delivery
workflows are green. The
repository also includes a bounded AI Gateway codec over an injected host
transport. An optional,
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
trusted injected custom transport, one shared retained workspace feeding the
current `file_info`, `glob_files`, `grep_files`, `list_files`, and `read_file`
candidate tool catalog under that same
retained identity, the existing file session store, and the
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

The delivered seventeenth bounded slice adds Linux/macOS library-only
`file_info`.
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
and compose with production at `f228c06`, where the initial 34 focused tests are
green. Review hardening composes at `b69ec4b`, bringing the independently owned
focused suite to 36 green tests plus five private unit tests. Required local
gates and all three replacement adversarial tracks are green on exact candidate
`4193ecc`. Documentation seal and integrated `main` SHA
`60dd54f273afc7e62fb4b3cc1fb1a347d739998b` is green under exact feature CI
run `32605071080` (successful retry attempt 2), feature benchmark-evidence run
`32605071063`, main CI run `32606050292`, and main benchmark-evidence run
`32606050294`; all four report that exact seal SHA. The benchmark successes are
delivery evidence only and make no product-performance claim. The overall
native-tool checklist stays open. This documentation-only commit is the final
delivery record, is explicitly exempt from another adversarial review after the
behavior was already green, and reports its own exact workflows at handoff. See
the [`file_info` contract](docs/file-info.md) and
[review record](docs/reviews/m03-file-info-review-01.md).

The eighteenth delivered bounded slice adds Linux/macOS library-only `glob_files`.
Strict effect-free preflight accepts a required glob pattern plus optional
search root and `matches`/`count` mode, prepares the distinct
`FilesystemAccess::EnumerateRecursive` capability at that normalized subtree,
and makes defaults explicit to allowed execution. Descriptor-relative,
no-follow iterative traversal includes hidden entries, never descends through
symlinks, reads no content, and fails without partial output if its entry,
name-byte, depth, candidate-path, or 8,388,608-step aggregate matcher-work cap
fires. Match output is the globally
bytewise-smallest sorted prefix under exact 100-path and 16 KiB aggregate path-
byte caps; count mode completes the same bounded scan and is exact. The
slice extends the composed host catalog to `file_info`, `glob_files`,
`list_files`, and `read_file`, with one retained workspace identity distributed
as the original descriptor plus three clones. Production, independent tests,
and documentation are composed. The first formal review at `1f5de6a` found a
high unmetered matcher-work defect; the checked 8,388,608-step fix, independent
both-mode regression, and all replacement local gates are green at exact
code-and-test head `4171a4a8811a98888b7e4e161281a1216564746f`. All three
replacement adversarial tracks are green on exact behavior SHA `523df858`.
Documentation seal and integrated `main` SHA
`35c853605077f2ac700f4be1dd79eabd2ace4dd4` passed exact feature CI run
`32610950593`, feature benchmark-evidence run `32610950594`, main CI run
`32611208411`, and main benchmark-evidence run `32611208415`; all four report
that exact seal SHA. Benchmark success is delivery evidence only and makes no
product-performance claim. This documentation-only commit is the final
`glob_files` delivery record, is explicitly exempt from another adversarial
review after behavior was green, and reports its own exact workflows at
handoff. Final documentation record
`f6aa458bb875d6cb26565adc878703fe140916d3` passed exact feature CI
`32611623653` and feature benchmark evidence `32611623655`. GitHub did not
materialize workflows for its first `main` event, so tree-identical
non-behavior successor `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4` passed
feature CI `32612424382` and feature benchmark evidence `32612424383`, was
fast-forwarded to `main`, and passed exact main CI `32612662260` and main
benchmark evidence `32612662203`. Neither docs-only record reopened behavior
review. See the
[`glob_files` contract](docs/glob-files.md) or
[review record](docs/reviews/m03-glob-files-review-01.md). The reviewed behavior changes
no CLI byte, benchmark workload, compatibility status, or performance claim,
and the combined native-tool checklist remains open.

The delivered nineteenth bounded slice adds Linux/macOS library-only `grep_files`
from exact base `f6aa458bb875d6cb26565adc878703fe140916d3`, with tree-identical
integration kickoff `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4`. Production,
independent tests, and maintained documentation are parallel, non-overlapping
components. Exact production `27eec2f` and initial independent-test `6eaee93`
components exist and initially compose through `9057feb` and `44e33d7`;
reference-host fixture fix `bdbb677` makes focused production/test composition
green. Documentation component `b04151a` produces first fully composed behavior
candidate `42e4793`; lint fix and exact local gates are green at `45ad91f`.
All three first-cycle formal tracks are **NOT GREEN** on exact candidate
`355a11a`. Remediation and exact replacement local gates are green at final
code/test precursor `275d263`. First replacement candidate `ae87bf1` is **NOT
GREEN** with one low correctness ordering finding, one low filesystem evidence-
wording finding, and medium-plus-low performance/cancellation findings.
Second production remediation `ac5d772` composes at `d672210`; second
documentation remediation `7ad0863` produces fully composed, exact local-gate-
green precursor `b498ba0`. `ae87bf1` remains historically **NOT GREEN**.
Formal second replacement candidate `5aeddc1` has correctness/API and
filesystem/robustness **GREEN** with zero findings; performance/concurrency is
**NOT GREEN** with one medium allocation-amplification finding and two low
documentation/evidence findings. Third production remediation `8777825`
composes at `ab1c133`; independent regression `dcf57ad` composes at `d7526d4`;
review-findings documentation `44afb23` composes at `f08c5f2`; lint follow-up
`1f13f9a` produces exact fully composed local-gate precursor `a8f6179`. Exact
Rust 1.94.1 formatting, warnings-denied workspace Clippy, 598 non-documentation
tests plus two doctests, 25 private native tests, 40 direct `grep_files` tests,
four engine tests, and diff checks are green on that precursor. Exact a8f
cross-target/dependency/link and compatibility/release validators are green.
Formal third-cycle candidate
`0bfe68a9692837187c057b5b4efa08ebe3dee058` has filesystem/robustness
**GREEN** with zero findings. Correctness/API and performance/concurrency are
**NOT GREEN** only for the same LOW documentation contract mismatch; reviewers
confirmed zero production defects. Isolated wording remediation
`993b618bf78d30f6a68f3b248b572e33e4de1126` composes at exact
`f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`; its documentation gates are
green, and its behavior tree remains `a8f6179` except for documentation. Formal
fourth-cycle exact behavior SHA
`8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is **GREEN** with zero findings
in all three fresh tracks: correctness/API, filesystem/robustness, and
performance/concurrency. Exact-SHA formatting, warnings-denied workspace
Clippy/tests, Linux/FreeBSD cross-target and WASI gates, two doctests, 25 private
tests, 40 direct `grep_files` tests, four engine tests, and the 58/420/270/0
documentation inventory are green. All historical findings are closed,
including the attempted-read-window storage wording. This documentation-only
seal is exempt from another adversarial review under the user's explicit
instruction. Documentation seal `0f48806310882caf3c668c72fe1b9d211cae744b`
is feature-green: CI run `32623585346` passed all six jobs and benchmark-
evidence run `32623585349` passed both jobs and artifacts, all for exact `0f`.
`main` was fast-forwarded without force from `f6ab594` to exact `0f`. Main CI
run `32623904784` is **GREEN** for exact `0f`: all six jobs and every step
passed without reruns. Main benchmark-evidence run `32623904800` is **GREEN**
on attempt 1 for exact `0f`: both jobs and every step passed, with two valid
non-expired exact-SHA artifacts retained. The `grep_files` slice is delivered;
the remaining native tools remain pending.
This final delivery record is documentation-only and exempt from adversarial
review; its own exact remote workflows are required after push and cannot be
self-recorded.
Strict effect-free preflight accepts exactly `pattern`, `path`, `include`,
`case_insensitive`, `mode`, `head_limit`,
`offset`, and `context_lines`, makes every default explicit, and prepares
`FilesystemAccess::SearchContent` at the normalized selected file or subtree.
The remediation contract requires fixed literal pattern-table work before root
resolution, one fully metered include compilation per call, full descendant-
path validation before allocation or filtering, reusable 64 MiB-bounded
continuation offsets, and selected-file filtering before content open. Slashful
selected-file rejection is charged and cancellation-checked; slashful candidate
splitting and both dynamic-programming branches retain fixed cancellation
checks. Allowed execution performs a bounded linear literal search with
optional ASCII case folding over eligible UTF-8 no-follow regular files reached
through the retained descriptor. One scan-local content buffer reads through an
8 KiB window, grows only to a 204,801-byte high-water ceiling, and logically
resets for reuse between files while preserving per-file and aggregate overflow
witnesses. Recursive and non-recursive include matching use injectable
cancellation checks with deterministic coverage. The tool reports exact
matching and eligible-text statistics in bounded `matches`,
`files_with_matches`, or `count` results, with same-buffer context and fixed
redacted errors. Candidate composition extends
the host to exactly five alphabetical tools—`file_info`, `glob_files`,
`grep_files`, `list_files`, and `read_file`—using the original retained
workspace descriptor plus four clones. It adds no CLI byte, benchmark workload,
compatibility status, performance claim, or fx-equivalence claim, and the
combined native-tool checklist stays open. See the
[`grep_files` contract](docs/grep-files.md) and
[review plan](docs/reviews/m03-grep-files-review-01.md).

The delivered twentieth through twenty-fourth slices add `write_file`,
`edit_file`, `delete_file`, `rename_file`, and `copy_file`. The twenty-fourth
slice extends the library host to exactly ten alphabetical tools by adding
`copy_file`, using the original retained workspace descriptor plus nine
identity-preserving clones. Its typed `FilesystemCopy` capability exposes both
canonical endpoints to policy. Approved execution confines both paths beneath
that retained root and streams at most 16 MiB through one 64 KiB buffer into a
private destination-parent stage before a single no-replace commit, bounded
postcommit verification, and destination-parent synchronization. It does not
overwrite, create parents, accept directory or symlink endpoints, allocate the
whole source, or broaden CLI authority. Seal `3bdd7cb` passed exact feature and
`main` CI/benchmark delivery gates with two artifacts in each benchmark run.
This description makes no complete fx-equivalence or performance claim. See
the [`copy_file` contract](docs/copy-file.md).

The frozen twenty-fifth `create_folder` contract accepts one strict canonical
confined workspace-relative path and recursively creates missing directory
components. It uses existing provider-neutral `FilesystemAccess::Create`
authority, requests mode `0755` while honoring host umask and ACL inheritance,
never follows symlinks or normalizes permissions afterward, never retries a
`mkdirat`, and never rolls back a created prefix. The first successful or
uncertain creation is the commit boundary; postcommit verification and
bottom-up durability are explicitly bounded. At this documentation-only
checkpoint the delivered host remains ten tools. Future behavior composition
must add `create_folder` after `copy_file` for eleven alphabetical tools and ten
descriptor clones. This is no implementation, delivery, performance, or
fx-equivalence claim. See the
[`create_folder` contract](docs/create-folder.md).

The project is not yet production-ready. See the exact
[CLI contract](docs/cli.md),
[`read_file` contract](docs/read-file.md),
[`list_files` contract](docs/list-files.md),
[`file_info` contract](docs/file-info.md),
[`glob_files` contract](docs/glob-files.md),
[`grep_files` contract](docs/grep-files.md),
[`write_file` contract](docs/write-file.md),
[`edit_file` contract](docs/edit-file.md),
[`delete_file` contract](docs/delete-file.md),
[`rename_file` contract](docs/rename-file.md),
[`copy_file` contract](docs/copy-file.md),
[`create_folder` contract](docs/create-folder.md), and
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
