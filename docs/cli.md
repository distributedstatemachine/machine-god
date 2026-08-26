# Command-line interface

In-progress slice 32 composes strict top-level `session <id> [--json]`
production and independent evidence. Initial composition was `852fec7`;
focused composition-gate remediation is exact
`c0c16a745943a97330223aafd4a6f6a7dce84ca6`, tree
`61bcf619fc9190a9a70ab3a9c643605c88ab1817`, under the normative
[`session` contract](session-cli.md). The command loads one exact
current-schema record through the separate
[`native inspection facade`](native-session-inspection.md) and renders only its
validated ID, incarnation ID, revision, next turn sequence, message count, and
top-level metadata-entry count. Transcript content, metadata keys/values,
`last`, `--id`, resume, replay, migration, recovery, workspace selection,
configuration, credentials, engine/provider construction, network, and runtime
are outside the slice. Parsing and ID validation precede effects; existing-root
inspection is no-create except for the file store's documented private `0600`
lock sidecar on a present record. Human/JSON output is assembled under 4,096
bytes. Non-exhaustive native categories fail closed to `Unavailable`, and help
bytes use `Inspect a saved session`. Focused exact-1.94.1 evidence is green for
12 native inspection tests, 56 CLI unit tests, 46 independent CLI process
tests, and native/CLI warnings-denied Clippy. Python, pinned-fx regeneration,
WASI/FreeBSD target, diff/no-unsafe, and exact-tree release-matrix checks are
also green. The full workspace exact gate passed on exact precursor
`fa099f75277f7ae23a3ac220e66356c45223d1a5`,
tree `64d6a72e66b6df78bc476dadd82ce3e911644b2d`, under exact Rust/Cargo 1.94.1;
documentation integrity is 85/146/620/81 with zero errors. The commit carrying
this gate record is the formal cycle-1 candidate once its exact same-SHA gate is
reconfirmed. Formal adversarial product review, remote workflows, `main`
integration, and delivery remain pending. There is no matching bootstrap
workload and no claim that the candidate is green or the
feature is delivered, performant, or equivalent. The delivered count remains
thirty-one; evidence is in the
[`live ledger`](reviews/m03-session-cli-review-01.md).

Delivered slice 31 adds a strict top-level `sessions [--json]`
surface under the frozen [`sessions` contract](sessions-cli.md). Only those two
forms are accepted, and parsing precedes environment and persistence effects.
The command lists the existing native store's bounded, ascending session IDs;
it does not invent workspace, title, preview, time, history, ranking, cursor, or
pagination data. A narrow native process facade safely opens an existing state
root without a workspace, engine, provider, credential, network transport, or
runtime. Missing selected roots are empty without creation. Existing canonical
records may cause the already-documented private `0600` lock sidecar to be
created during validation. Human and compact JSON representations are built
before output and capped at 16 KiB. The pinned `sessions-json` workload becomes
implemented but remains non-equivalent, not measured, and claim-ineligible.
The [`live review ledger`](reviews/m03-sessions-cli-review-01.md) records exact
candidate `9448738` and its rejected `0/0/0/3` cycle-1 verdict; remediation is
composed in exact candidate `a527652`, tree `0249dd0`, which passed three fresh
exact-SHA cycle-2 reviews at `0/0/0/0` each. Review-exempt seal `b5b9116`, tree
`3e61754`, passed exact feature CI `32939742230`, feature benchmark evidence
`32939742231`, main CI `32940279028`, and main benchmark evidence `32940279005`;
`main` was fast-forwarded without force. The delivered count is thirty-one.

The `machine-god` binary is the thin native reference host for the embeddable
engine. This page defines the exact implemented Milestone 03 config/status and
permissions surfaces. Status inspects process environment and filesystem
metadata without
parsing configuration. Permissions loads the existing strict native
configuration exactly once and read-only. Neither path creates directories,
writes files, starts the engine, or grants runtime authority.
The separate integrated [`native configuration schema v3`](configuration.md)
does not change that boundary or any CLI byte documented below. Its production
implementation, independent tests, local gates, and all three adversarial tracks
are green on exact behavior SHA `35ce591e8ca6a8fef94485ff85d3e9c1397130a6`.
The slice is integrated on `main` through final delivery record
`f840576af241c58d1e55399e66ba92f7770cd50c`; exact main CI run `32583871385`
and benchmark-evidence run `32583871368` are green. Provider, transport, model,
and credential-source config fields remain invisible to this metadata-only
surface. The separate fourteenth composed
[`native root-selection candidate`](native-root-selection.md) likewise changes
no invocation or output byte. Status does not call selection or preparation and
remains metadata-only and no-create.

Bounded slice 30 adds strict top-level `doctor [--json]` under the frozen
[`doctor` contract](doctor-cli.md). The slice is **DELIVERED** from exact base
`f82ce46736f7bac4154da508e3b768d0b9248e15`.
Exact candidate `15f8176`, tree `8278a77`, passed the complete exact-1.94.1
local gate and three fresh review tracks at `0/0/0/0` each. Review-exempt seal
`345f812`, tree `8899849`, passed feature CI `32933464234`, feature benchmark-
evidence `32933464047`, main CI `32933879888`, and main benchmark-evidence
`32933879930`; `main` was fast-forwarded without force from the delivered base.
It reports exactly
four ordered read-only checks, builds either representation under an inclusive
4,096-byte cap, and creates nothing. A check-level `fail` remains exit 0;
invalid arguments exit 2, while render and output-write failures exit 1. The
delivered count is thirty. Bootstrap `doctor-json` is implemented but
non-equivalent, not measured, and claim-ineligible against pinned fx
`b1774fbf6c7602b503026f96f6e960e946c692ef`; no performance or equivalence
claim is made.

The delivered twenty-eighth bounded slice adds top-level
`permissions [--json]` under the frozen boundary in
[`permissions-cli.md`](permissions-cli.md). It loads the existing strict native
configuration read-only and reports ask-only permission configuration without
constructing an engine or inventing persistent rule or runtime grant state.
Its separate production, independent-evidence, and documentation components
are composed in this feature change. Exact cycle-5 reviewed candidate
`0b13944d19cfb33b4542d82d74c302669817c1af`, tree
`2ea72e810f07ed8ca2d4e8647fa713088477d8b5`, passed the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback. The gate passed 928
non-documentation executions and two doctests, focused native configuration
25+29 and CLI 6+19, pinned-fx compatibility and all 31 generator tests,
documentation integrity 76/110/548/391 with zero errors, no dependency or
unsafe delta, and the full release matrix. The fresh 368,944-byte binary has
SHA-256 `8756c7801285f1b09cad9a8b8ce47700a44127dec68ef2b0613e6a5dcecad45e`.
Correctness/API, native config/error lifecycle, and performance/CLI portability
each reported 0/0/0/0 in three fresh reviews; the union is zero and the
behavior candidate is **GREEN**. Documentation-only seal
`3e41cc6b90adb34d62aec21c6d03729d59ca0c1b`, tree
`bd74a96c4952c2eb1e15372f4ab716a76bba91a9`, is exempt from redundant
adversarial review. Exact feature CI `32891031065`, feature benchmark-evidence
`32891031147`, main CI `32891614025`, and main benchmark-evidence `32891614060`
are green on that exact seal SHA. `main` was fast-forwarded without force from
`8d8ecc7a37f866251d4047c01acdf1bbd485f4da`, and each benchmark run retains
exactly two unexpired exact-SHA artifacts for 90 days. M03 remains in progress
with twenty-nine delivered slices. This delivery makes no product-performance
or fx-equivalence claim. The final delivery-record commit is documentation-only
and review-exempt; its own exact feature and `main` workflows will be reported at
handoff rather than claimed here.

The delivered twenty-ninth bounded slice adds top-level `models [--json]` in
[`models-cli.md`](models-cli.md). It
preserves parse-before-effects, loads strict configuration exactly once, uses
the existing two-name native credential policy, and composes a provider-neutral
core catalog trait with one fixed bounded native Gateway GET provider. Native
owns access/fallback, deadline, parsing, bounds, and ordering; the CLI owns
rendering and the output cap. Its successful JSON starts with `kind` and
deliberately does not prepend the
identity/status fields from other commands. It owns no engine, selected
generation model, prompt, permission, workspace, state, session, cache, or
write effect. Core/native/CLI components are present through local feature
commit `e84ed2a46b1ac5fe7428414375609af562c65105`; native terminal-precedence
remediation is `52e9b7d74f3979f7f7f55387243e96bd78773fe3`, and 35 focused independent
native tests are present at `12263afa458e48f2963ae3d0e3db5cf219f8bdf6`.
Exact cycle-1 candidate `6277aa3`, tree `b5e2445`, passed its local gate but was
rejected with two medium and six low findings. Remediation is composed at
`02c9f86`, `d2890c3`, and `06c9408`; focused native evidence now totals 36 and
CLI unit evidence 18. Exact cycle-2 behavior candidate `2ea9d94`, tree
`3a948b2`, passed the complete replacement gate but was rejected with one high,
one medium, and one low finding. Parser and HTTP lifecycle remediation is
composed at `9cf8c74`, `8187b12`, and `499af85`. Pre-review gate attempt
`c011398`, tree `4ac4e5b`, was rejected by its DNS-config lifecycle audit;
remediation is composed at `d9922ef` and `e5248b1`. Exact cycle-3 candidate
`2cecc921`, tree `8c0d235`, passed its complete gate but formal review rejected
it with a deduplicated union of one medium and three low findings. Documentation,
private bounded-DNS, and Android fail-closed remediation is composed at
`f80bd056`, `b6cf4cb`, and `bd47461`. Formal cycle 4 rejected exact candidate
`57d2ac2`, tree `d30bb656`, after its complete exact-1.94.1 replacement gate.
Its raw overlap-deduplicated union was 0 blocker, 0 high, 1 medium, and 2 low;
after prior sealed dispositions, 0 blocker, 0 high, 1 medium, and 1 low remained
unresolved at verdict collection. Topology documentation is fixed at `268d35a`;
signal/output-lifecycle remediation is integrated at exact `aa60db1`, tree
`278fa365`. Exact cycle-5 candidate `27c75f4`, tree `5e40b24`, passed the
complete exact-1.94.1 replacement gate without fallback. Three fresh formal
reviews each reported 0 blocker, 0 high, 0 medium, and 0 low findings. Their
deduplicated union is zero, so the behavior candidate is **GREEN**. Exact
review seal `2064084`, tree `33818a4`, passed feature benchmark run
`32923421739`; feature CI `32923421679` failed solely on one Linux test-helper
Clippy diagnostic, and no integration occurred. Exact test-only cycle-6
replacement candidate `831d38c8`, tree `a92acc14`, passed the complete
replacement gate and three fresh formal reviews with zero findings. Review-
exempt delivered seal `bacc5c3dbc2bf094cca12102030d21f468f11e7a`, tree
`da3183a3368273c2b34324a5f33266dfe5644a0d`, passed exact feature CI
`32925681006`, feature benchmark-evidence `32925681009`, main CI `32926242609`,
and main benchmark-evidence `32926242564`. `main` was fast-forwarded without
force from `1de3b7eddf6a4d9046d48098defecf6bfa336442`; each benchmark run retains
exactly two unexpired exact-SHA artifacts for 90 days. M03 remains in progress
with twenty-nine delivered slices. This delivery makes no product-performance,
speed, latency, memory, binary-size-improvement, catalog-equivalence,
compatibility-promotion, or fx-equivalence claim. The final delivery-record
commit is documentation-only and review-exempt; its own exact feature and
`main` workflows will be reported at handoff rather than claimed here.

The delivered by-ID native lifecycle and delivered sixteenth
[`native session-listing extension`](native-session-listing.md) are also
library-only. They add no `session`, `sessions`, `resume`, `replay`, workspace,
or slash command and change none of the accepted invocations, output bytes,
diagnostics, or exit statuses below. In particular, the listing's IDs-only
bounded result is not exposed as fx-compatible CLI output. That first formal
candidate was not green; all three review tracks and exact feature and `main`
delivery workflows are green on its replacement lineage.

The delivered seventeenth [`file_info` slice](file-info.md) is likewise
library-only.
It adds no `file_info`, workspace, or slash command and changes none of the
accepted invocations, output bytes, diagnostics, or exit statuses below. Its
production and independent tests compose at `f228c06`, where all 34 focused
initial tests are green. Review hardening brings the focused total to 36 plus
five private unit tests at `b69ec4b`. All three replacement adversarial tracks
are green on exact candidate `4193ecc`. Documentation seal and integrated
`main` SHA `60dd54f273afc7e62fb4b3cc1fb1a347d739998b` passed exact feature CI
run `32605071080` on successful retry attempt 2, feature benchmark-evidence run
`32605071063`, main CI run `32606050292`, and main benchmark-evidence run
`32606050294`; all four report that exact seal SHA. The benchmark successes are
delivery evidence only, not a product-performance claim. This documentation-only
commit is the final delivery record, is explicitly exempt from another
adversarial review after behavior was green, and reports its own exact workflows
at handoff. Its metadata result is not exposed as CLI output.

The delivered eighteenth [`glob_files` slice](glob-files.md) is also library-only.
It adds no `glob_files`, workspace, or slash command and changes none of the
accepted invocations, output bytes, diagnostics, or exit statuses below. Its
bounded path/count result is not exposed as CLI output. Production, independent
tests, documentation, and composition are present. The first review at
`1f5de6a` found a matcher-work bound defect; its fix, regression, and replacement
local gates are green at `4171a4a`. All three replacement review tracks are
green on exact behavior SHA `523df858`. Documentation seal and integrated
`main` SHA `35c853605077f2ac700f4be1dd79eabd2ace4dd4` passed exact feature CI
`32610950593`, feature benchmark evidence `32610950594`, main CI `32611208411`,
and main benchmark evidence `32611208415`. The slice makes no compatibility or
performance claim; benchmark success is delivery evidence only. Final docs
record `f6aa458bb875d6cb26565adc878703fe140916d3` passed feature CI
`32611623653` and benchmark evidence `32611623655`. Because GitHub did not
materialize workflows for its first `main` event, tree-identical non-behavior
successor `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4` passed feature CI
`32612424382` and benchmark evidence `32612424383`, then exact main CI
`32612662260` and benchmark evidence `32612662203` after fast-forward.

The nineteenth [`grep_files` candidate](grep-files.md) is also library-only.
It adds no `grep_files`, workspace, or slash command and changes none of the
accepted invocations, output bytes, diagnostics, or exit statuses below. Its
bounded structured match, file, and count results are not exposed as CLI
output. The candidate starts from exact base
`f6aa458bb875d6cb26565adc878703fe140916d3`; the tree-identical integration
kickoff is `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4`. Production, independent
tests, and documentation are parallel, non-overlapping components. Exact
production `27eec2f` and initial test `6eaee93` components exist and initially
compose through `9057feb` and `44e33d7`; fixture fix `bdbb677` makes focused
production/test composition green. Documentation component `b04151a` produces
fully composed behavior `42e4793`; lint fix and exact local gates are green at
`45ad91f`. All three first-cycle tracks are **NOT GREEN** on exact `355a11a`;
remediation and exact replacement local gates are green at final code/test
precursor `275d263`. First replacement candidate `ae87bf1` is **NOT GREEN**
across all three tracks. Second-fix production and documentation compose through
`ac5d772`, `d672210`, `7ad0863`, and exact local-gate-green precursor
`b498ba0`. Formal second replacement candidate `5aeddc1` has correctness/API
and filesystem/robustness **GREEN** with zero findings and
performance/concurrency **NOT GREEN** with one medium allocation-amplification
finding and two low documentation/evidence findings. Third production
remediation `8777825` composes at `ab1c133`; independent regression `dcf57ad`
composes at `d7526d4`; review-findings documentation `44afb23` composes at
`f08c5f2`; lint follow-up `1f13f9a` produces exact fully composed local-gate
precursor `a8f6179`. Exact Rust 1.94.1 formatting, warnings-denied workspace
Clippy, 598 non-documentation tests plus two doctests, 25 private native tests,
40 direct `grep_files` tests, four engine tests, and diff checks are green.
Exact a8f cross-target/dependency/link and compatibility/release validators are
green. Formal third-cycle candidate
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
self-recorded. It makes no compatibility or performance claim.

## Accepted invocations

The accepted argument forms are exactly:

```text
machine-god
machine-god help
machine-god --help
machine-god -h
machine-god --version
machine-god -V
machine-god doctor
machine-god doctor --json
machine-god models
machine-god models --json
machine-god permissions
machine-god permissions --json
machine-god sessions
machine-god sessions --json
machine-god status
machine-god status --json
```

Bare invocation, `--version`, and `-V` write this exact identity to stdout:

```text
machine-god 0.1.0 (engine API 1)
```

The output ends in one LF. Bare invocation intentionally remains the original
identity behavior.

`help`, `--help`, and `-h` write this exact stdout, including the final LF:

```text
machine-god 0.1.0
Embeddable coding-agent engine

Usage:
  machine-god
  machine-god help
  machine-god doctor [--json]
  machine-god models [--json]
  machine-god permissions [--json]
  machine-god sessions [--json]
  machine-god status [--json]

Commands:
  help         Show this help
  doctor       Run local health and preflight checks
  models       List available models
  permissions  Show the permission mode and rules
  sessions     List saved sessions
  status       Show configuration and runtime information

Options:
  -h, --help       Show this help
  -V, --version    Show version
```

## Doctor output

`machine-god doctor` writes one count summary followed by exactly four ordered
check lines:

```text
[doctor] ok=N warn=N fail=N
[<status>] config: <detail>
[<status>] credential: <detail>
[<status>] state: <detail>
[<status>] platform: <detail>
```

`machine-god doctor --json` writes one compact object with fixed top-level key
order `kind,ok_count,warn_count,fail_count,checks`; each check object uses
`name,status,detail`. Statuses are only `ok`, `warn`, and `fail`; the three
counts sum to four and match the array. Both forms end in LF and the complete
output is capped at 4,096 bytes including that LF. Exact mappings and examples
are in [`doctor-cli.md`](doctor-cli.md).

Every complete report exits 0 even when a diagnostic is `fail`. Render failure
exits 1 with `machine-god doctor: could not render report`; stdout write failure
exits 1 with `machine-god: failed to write output`. The command is read-only,
creates no missing root, and emits no path, runtime, session, workspace, or
model state.

## Models output

`machine-god models` writes this nonempty shape, with one ID line per native-
sorted ID:

```text
[models] N available
 - <id-1>
 - <id-2>
```

An empty catalog instead writes `[models] no models returned by gateway`.
Missing-credential public success appends exactly `[models] Using the public
model catalog; set VERCEL_OIDC_TOKEN or AI_GATEWAY_API_KEY to include private
models.`. Authenticated-401/403 public fallback appends exactly `[models]
Gateway authentication was rejected; showing the public model catalog.`.
Authenticated success appends nothing. Each selected representation ends in
one LF; the sentence wrapping in this paragraph is not output whitespace.

`machine-god models --json` writes one compact object in fixed key order with a
final LF and no identity prefix:

```json
{"kind":"models","count":2,"shown_count":2,"more_count":0,"private_models_hidden":false,"ids":["provider/a","provider/b"]}
```

The complete success output is built under an inclusive 64 KiB cap before its
first stdout write. Exact failure shapes, codes, channels, credential/fallback
behavior, and resource bounds are normative in
[`models-cli.md`](models-cli.md). Exact cycle-6 replacement `831d38c8`, tree
`a92acc14`, is delivered under exact seal `bacc5c3`, tree `da3183a`, and the
feature and `main` workflow evidence recorded above.

The native command keeps its Ctrl-C listener and, on Unix, SIGTERM listener
actively driven through rendering and every synchronous success or failure
write. A first signal during provider work keeps the documented graceful
cancellation result. A later observed signal, or the first signal after the
provider becomes terminal, exits promptly with 130 for Ctrl-C or 143 for Unix
SIGTERM even when output is backpressured. Normal output joins the bounded
signal guardian after a final runtime-driver drain and signal recheck, so a
queued delivery cannot lose to fast output completion. Exceptional closure of
an installed signal stream fails stop with exit 1 before output because
Tokio's process handler cannot safely be left installed without a driven
listener.

## Permissions output

`machine-god permissions` writes four lines with a final LF:

```text
machine-god 0.1.0 (engine API 1)
permission_mode: ask
persistent_rules: unsupported
runtime_grants: unavailable
```

`machine-god permissions --json` writes this exact compact object in fixed key
order, followed by one LF:

```json
{"name":"machine-god","version":"0.1.0","engine_api_version":1,"kind":"permissions","permission_mode":"ask","persistent_rules_supported":false,"runtime_grants_available":false}
```

Argument validation completes before any configuration access. A valid
invocation calls `load_process_config()` exactly once. A missing file or
unavailable configuration location uses the safe built-in schema-v3
configuration; valid strict v1, v2, or v3 configuration reports `ask` and is
never rewritten. The command observes no provider, transport, model, credential
source, config path, or state path.

Every configuration-load failure exits 1, writes no stdout, and writes this
fixed redacted diagnostic with a final LF:

```text
machine-god: failed to load configuration
```

The fields reporting unsupported persistent rules and unavailable runtime
grants are honest capability statements, not empty policy databases. The
command creates no engine, permission prompt, session store, state root, Tokio
runtime, credential discovery, network transport, rule store, or grant store.

On supported Unix targets, configuration loading opens the selected final path
with `O_NOFOLLOW` and nonblocking behavior and authoritatively requires a
regular file. Hardened non-Unix opening remains deferred.

Cycle 1 rejected the original candidate because configuration reads could retry
`Interrupted` indefinitely and because the config-only command snapshotted an
unused `XDG_STATE_HOME` value. The cycle-1 replacement bounds the 16th
cumulative interruption with fixed `Unreadable` and never requests
`XDG_STATE_HOME`. Cycle 2 found that it still reads and stores `HOME` eagerly
when nonempty `XDG_CONFIG_HOME` already decides selection. The composed cycle-2
replacement reads `XDG_CONFIG_HOME` first and reads `HOME` only when XDG is
missing or empty; a nonempty valid, invalid-relative, or non-Unicode XDG value
neither reads nor falls back to `HOME`. The cycle-3 and cycle-4 gates confirmed
this behavior; cycle 4 was rejected only for separate ambiguous delivery
terminology in `security.md`. Exact cycle-5 candidate `0b13944d` then passed
the complete replacement gate and three fresh reviews with zero findings at
every severity.

## Sessions output

`machine-god sessions` writes `[sessions] no saved sessions` for an empty
complete observation. A nonempty result writes `[sessions] N saved` followed by
one ` - <id>` line per ascending session ID. Every truncated result appends
`[sessions] listing incomplete: a resource limit was reached`. JSON uses fixed
top-level key order `kind,count,truncated,sessions`, and each array element has
the sole current key `id`:

```json
{"kind":"sessions","count":2,"truncated":false,"sessions":[{"id":"alpha"},{"id":"beta"}]}
```

Both forms end in one LF and are assembled under a 16 KiB ceiling before the
first success write. The strict grammar, redacted error/channel boundary,
state-root policy, native scan limits, permitted lock-sidecar write, and pinned-
fx differences are normative in [`sessions-cli.md`](sessions-cli.md). Slice 31
is still in progress; this exact-byte contract is not yet a delivery claim.

## Status output

`machine-god status` writes four lines with a final LF:

```text
machine-god 0.1.0 (engine API 1)
permission_mode: ask
config_file: state=<state> path=<JSON-string-or-null>
state_directory: state=<state> path=<JSON-string-or-null>
```

Even in human output, a present path is encoded as a JSON string. Quotes,
backslashes, C0/C1 controls, Unicode line/paragraph separators, and Unicode
bidirectional-formatting controls are escaped. An unresolved path is the
unquoted token `null`.

`machine-god status --json` writes one compact JSON object in this fixed key
order, followed by one LF:

```json
{"name":"machine-god","version":"0.1.0","engine_api_version":1,"permission_mode":"ask","config_file":{"path":null,"state":"unavailable"},"state_directory":{"path":null,"state":"unavailable"}}
```

The example shows unavailable paths. The exact structural form is:

```text
{"name":"machine-god","version":"0.1.0","engine_api_version":1,"permission_mode":"ask","config_file":{"path":<JSON-string-or-null>,"state":"<state>"},"state_directory":{"path":<JSON-string-or-null>,"state":"<state>"}}
```

Config-file state is one of `file`, `missing`, `not_file`, `inaccessible`,
`unavailable`, or `invalid_environment`. State-directory state is one of
`directory`, `missing`, `not_directory`, `inaccessible`, `unavailable`, or
`invalid_environment`. A valid resolved path is reported even when its state is
missing, inaccessible, or the wrong kind. The path is `null` only for
`unavailable` or `invalid_environment`.

Permission mode is always `ask` in this slice. It reports the native host's
fixed safe default; this CLI path constructs no permission prompt or
permission-gated native tool. Status does not load a legacy v1 or v2 config or
the current v3 config and therefore does not report its observable schema version,
provider, transport, model, or credential source.

## Config and state locations

Configuration and state use only the `machine-god` namespace:

- a nonempty `XDG_CONFIG_HOME` selects
  `$XDG_CONFIG_HOME/machine-god/config.json`;
- otherwise, a nonempty `HOME` selects
  `$HOME/.config/machine-god/config.json`;
- a nonempty `XDG_STATE_HOME` selects `$XDG_STATE_HOME/machine-god`;
- otherwise, a nonempty `HOME` selects
  `$HOME/.local/state/machine-god`.

An empty XDG value is treated as absent and falls back to `HOME`. If a selected
nonempty XDG root is relative or not Unicode, that location is
`invalid_environment`; the CLI does not fall back to `HOME`. A selected
nonempty `HOME` must likewise be absolute Unicode. Absent or empty `HOME` makes
a fallback location `unavailable`.

Status snapshots `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, and `HOME` because it
reports both locations. The rejected cycle-1 permissions candidate also
snapshotted all three even though it used only config resolution. Its composed
config-only replacement never requests `XDG_STATE_HOME`, but cycle 2 found its
`HOME` request eager. The composed replacement reads `XDG_CONFIG_HOME` first,
reads `HOME` only when XDG is missing or empty, and does not read or fall back
to `HOME` when a nonempty valid, invalid-relative, or non-Unicode XDG value is
selected.

Inspection calls `symlink_metadata` on each final path. A final symlink is not
followed: a config symlink reports `not_file`, and a state-directory symlink
reports `not_directory`. The command does not canonicalize paths, parse
`config.json`, follow a final symlink, create missing locations, or write any
state.

## Errors and exit status

Valid invocations exit zero after writing their output. Any other argument
sequence, including a non-UTF-8 argument, writes no stdout, exits 2, and writes
this exact stderr with a final LF:

```text
machine-god: invalid arguments
Usage: machine-god [help | --help | -h | --version | -V | doctor [--json] | models [--json] | permissions [--json] | sessions [--json] | status [--json]]
```

An output-write failure exits 1 and uses this fixed diagnostic on stderr:

```text
machine-god: failed to write output
```
