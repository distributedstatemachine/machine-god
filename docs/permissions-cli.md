# Native `permissions` CLI contract

Status: **IN PROGRESS — CONTRACT FROZEN; IMPLEMENTATION PENDING**.

This is the proposed twenty-eighth bounded Milestone 03 slice. It starts from
exact delivered base `8d8ecc7a37f866251d4047c01acdf1bbd485f4da`, tree
`e1508d91edfb524df470cdf7c9b3112c4d145e4a`. The slice adds one read-only
top-level command to the thin native host. It does not complete the M03 CLI
inventory and makes no product-performance or fx-equivalence claim.

## Command grammar

The only accepted forms are:

```text
machine-god permissions
machine-god permissions --json
```

`--json` is accepted exactly once and only after `permissions`. Unknown,
reordered, repeated, additional, or non-Unicode arguments fail through the
existing invalid-arguments boundary: exit code 2, empty standard output, and
the one exact usage diagnostic on standard error. Argument validation completes
before configuration is inspected.

The general `help`, `--help`, and `-h` output lists the new command. This slice
does not add command-local help or a `/permissions` interactive slash command.

## Authority and loading

After successful parsing, `permissions` calls `load_process_config()` exactly
once and observes only the validated `NativeConfig::permission_mode()`. The
loader remains bounded to 64 KiB, synchronous, read-only, no-follow for the
selected final configuration path, and redacted on failure.

A missing file or unavailable configuration location uses the safe built-in
configuration. Valid strict schema-v1, schema-v2, and schema-v3 files report the
same currently supported `ask` mode without rewriting any byte. An invalid
selected environment, wrong file type, unreadable or oversized file, malformed
configuration, or unsupported schema version fails closed with exit code 1,
empty standard output, and exactly:

```text
machine-god: failed to load configuration
```

The diagnostic does not disclose the error kind, path, configuration content,
model, provider, transport, credential source, or operating-system detail.

The command does not inspect or create the state root; construct an engine,
provider, transport, credential source, permission prompter, session store, or
Tokio runtime; read a credential; make a network request; prompt; persist a
rule; or cache a grant.

## Exact output

Human output is exactly:

```text
machine-god 0.1.0 (engine API 1)
permission_mode: ask
persistent_rules: unsupported
runtime_grants: unavailable
```

JSON output is one compact object with stable key order and one final LF:

```json
{"name":"machine-god","version":"0.1.0","engine_api_version":1,"kind":"permissions","permission_mode":"ask","persistent_rules_supported":false,"runtime_grants_available":false}
```

`persistent_rules_supported: false` means this slice does not expose or manage
identity-safe persistent policy. `runtime_grants_available: false` means the
read-only command has no live engine or permission-handler snapshot. Neither
field asserts that an unobserved collection is empty.

If writing successful output fails, the existing output boundary returns exit
code 1 and the fixed `machine-god: failed to write output` diagnostic. No partial
success is claimed.

## Compatibility and deferrals

Pinned fx exposes a scenario named `permissions [--json]`. Machine-god aligns
with that read-only discovery scenario but intentionally uses its own exact
output and supports only validated mode `ask`. The combined top-level CLI
compatibility surface remains planned; this partial slice does not promote the
generated inventory.

Deferred work includes modes beyond `ask`, persistent rule schemas, identity-
safe grants, allowlists, sandbox policy, mutation, interactive `/permissions`,
and live session-grant introspection. Those authority-bearing surfaces belong
to the frozen M04 or later boundary, not the thin CLI.

## Required evidence

Independent tests must cover:

- exact grammar, updated global help/usage, non-Unicode arguments, exit codes,
  standard streams, JSON key order, and final LF;
- missing and unavailable configuration defaults plus valid v1/v2/v3 files;
- invalid environment, symlink/wrong-kind, unreadable, oversized, malformed,
  and unsupported-version failures with fixed redaction;
- byte-identical configuration files and absence of newly created config/state
  roots;
- unchanged identity/version/status behavior except the intentional global
  help and invalid-usage additions; and
- freshly built release-binary human, JSON, invalid-config, no-create, and
  no-rewrite smokes.

The exact implementation must pass the repository-required Rust 1.94.1 gate,
then one immutable candidate SHA and tree must receive three fresh independent
ordinary product reviews: correctness/API, native config/error lifecycle, and
performance/CLI portability. All findings are fixed and reviewed again until
all three tracks report zero findings. Exact feature and `main` CI plus
benchmark-evidence workflows remain delivery requirements.
