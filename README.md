# machine-god

`machine-god` is an experimental, embeddable coding-agent engine written in Rust.

The engine is the primary product. The command-line application is its native
reference host. Development status, architecture, compatibility, security, and
performance evidence live in [`docs/`](docs/README.md).

Milestones 01 and 02 are complete, and Milestone 03 is in progress. The
repository includes the provider-neutral streaming engine, its bounded durable
tool loop, a deterministic testkit, read-only native configuration/status
discovery and loading, and capability-aware tool preflight before permission
policy. It also includes bounded Unix-only `read_file` and one-level
`list_files` library capabilities rooted in host-injected workspaces, plus a
bounded AI Gateway codec over an injected host transport. An optional,
native-only HTTPS transport for that codec is the seventh integrated bounded
slice. It uses one pinned production endpoint and an explicitly injected,
redacted bearer token.
An eighth bounded candidate implements a Unix file-backed session store under
an explicit host-opened root. Its exact behavior SHA and local gates are
adversarially green; feature-branch remote gates and `main` integration remain
pending. Credential discovery, provider/CLI wiring,
permission prompting, broader session lifecycle features, the remaining native tools, and
compatibility work remain planned; the project is not yet production-ready. See
the exact [CLI contract](docs/cli.md),
[`read_file` contract](docs/read-file.md),
[`list_files` contract](docs/list-files.md), and
[AI Gateway codec](docs/ai-gateway.md) plus
[native HTTP transport](docs/ai-gateway-http.md) contracts, and the normative
[native file session-store candidate](docs/session-store.md).

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
