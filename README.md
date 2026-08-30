# machine-god

`machine-god` is an experimental coding-agent engine written in Rust. The
embeddable asynchronous engine is the product; the command-line binary is a
small native reference host for the surfaces that have been integrated so far.

The project is under active development. The single source of truth for current
scope, milestone state, and delivery evidence is the
[current delivery state](docs/implementation-plan.md#current-delivery-state).

## What is here

- `machine-god-core`: provider-neutral orchestration, streaming model events,
  sessions, tool preparation, permission decisions, cancellation, and bounds.
- `machine-god-native`: explicit operating-system, persistence, network,
  credential, prompt, and native-tool adapters.
- `machine-god-cli`: the `machine-god` binary. It currently exposes bounded
  help, status, configuration, model-catalog, and session-inspection commands;
  it is not yet the complete interactive agent UI.
- `machine-god-testkit`: deterministic providers, stores, tools, permission
  handlers, and event sinks for embedders and tests.

Core has no ambient filesystem, process, environment, credential, clock,
randomness, or network access. A host must inject every authority-bearing
component. See the [architecture](docs/architecture.md),
[security boundaries](docs/security.md), and
[native reference-host composition](docs/native-reference-host.md).

## Build and explore

The workspace requires Rust and Cargo 1.94.1.

```sh
cargo +1.94.1 build --workspace
cargo +1.94.1 run -p machine-god-cli -- --help
```

Build the optimized binary with:

```sh
cargo +1.94.1 build --release --locked -p machine-god-cli
./target/release/machine-god --help
```

Native capabilities are deliberately platform- and feature-gated. The complete
reference-host library composition is available on Linux and macOS with the
`ai-gateway-http` feature; individual contracts document narrower boundaries.

## Development

Read [AGENTS.md](AGENTS.md) and the
[implementation plan](docs/implementation-plan.md) before changing code. The
required local gate is:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Run focused tests before the full workspace and exercise user-visible behavior
through a freshly built release binary. Feature delivery also requires three
fresh product reviews and exact-commit remote CI as described in the plan.

## Documentation

Start at the [documentation index](docs/README.md). Detailed behavior belongs
in stable contract documents near the corresponding subsystem. Review ledgers
under `docs/reviews/` are historical evidence, not a second live status system.

The upstream comparison is pinned by `benchmarks/upstream.lock`. Zig is used
only to build that upstream fx input; machine-god itself is written and shipped
as Rust. The benchmark wrapper re-hashes a cached official archive, extracts a
fresh exact Zig toolchain for one run, and removes that extraction afterward
without changing the system toolchain:

```sh
python3 benchmarks/with_zig.py -- --runs 30
```
