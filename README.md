# machine-god

`machine-god` is an experimental, embeddable coding-agent engine written in Rust.

The engine is the primary product. The command-line application is its native
reference host. Development status, architecture, compatibility, security, and
performance evidence live in [`docs/`](docs/README.md).

Milestone 01's repository, documentation, CI, compatibility inventory, and
pinned upstream benchmark harness with non-product bootstrap evidence are
complete. Milestone 02's provider-neutral engine is in progress; the project is
not yet production-ready.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

The project is licensed under Apache-2.0. It is inspired by
[`vercel-labs/fx`](https://github.com/vercel-labs/fx), whose pinned comparison
revision is recorded in `benchmarks/upstream.lock`.
