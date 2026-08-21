# machine-god

`machine-god` is an experimental, embeddable coding-agent engine written in Rust.

The engine is the primary product. The command-line application is its native
reference host. Development status, architecture, compatibility, security, and
performance evidence live in [`docs/`](docs/README.md).

Milestones 01 and 02 are complete, and Milestone 03 is in progress. The
repository includes the provider-neutral streaming engine, its bounded durable
tool loop, a deterministic testkit, read-only native configuration/status
discovery and loading, and capability-aware tool preflight before permission
policy. It also includes a bounded Unix-only `read_file` library capability
rooted in a host-injected workspace. CLI wiring and permission prompting,
concrete providers, durable native sessions, the remaining native tools, and
compatibility work remain planned; the project is not yet production-ready.
See the exact [CLI contract](docs/cli.md) and
[`read_file` contract](docs/read-file.md).

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
