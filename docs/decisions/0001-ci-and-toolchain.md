# ADR 0001: Reproducible CI actions and toolchains

- Status: Accepted
- Date: 2026-08-20

## Context

The workspace declares Rust 1.94.1 as its supported compiler and CI toolchain.
The original workflows used JavaScript actions whose bundled runtime was Node
20, which reached end of life and is being retired by GitHub-hosted runners.
Those workflows also built `cargo-deny` and `cargo-audit` in separate ephemeral
jobs, repeating runner and dependency setup without increasing their isolation
from product builds.

The exact Rust 1.94.1 rustup installation in this checkout has also exhibited
damage even when the installed `stable` channel contains the same 1.94.1 Rust
and Cargo releases. A local fallback must not silently turn the pinned compiler
gate into a floating-stable gate.

## Decision

CI installs and invokes Rust 1.94.1 explicitly. Local verification uses
`+1.94.1` by default. It may use `+stable` only when both
`rustc +stable --version` and `cargo +stable --version` report release 1.94.1
exactly, and the substitution is recorded with the check results. Once stable
moves beyond 1.94.1, it is not an acceptable substitute.

Third-party JavaScript actions are pinned to complete immutable commit SHAs.
The reviewed pins are:

| Action | Release | Commit |
| --- | --- | --- |
| `actions/checkout` | [v7.0.1](https://github.com/actions/checkout/releases/tag/v7.0.1) | `3d3c42e5aac5ba805825da76410c181273ba90b1` |
| `actions/upload-artifact` | [v7.0.1](https://github.com/actions/upload-artifact/releases/tag/v7.0.1) | `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` |

Both commits are GitHub-verified release commits, and their `action.yml`
manifests declare `runs.using: node24`. Every checkout retains
`persist-credentials: false`, and workflows retain only read access to repository
contents.

Dependency policy and vulnerability audit run in one security job, still
isolated from the quality and target-check jobs. The job installs exact locked
versions of both tools with Rust 1.94.1 from the runner temporary directory. A
shared temporary Cargo target lets the second install reuse compatible registry
and build artifacts; no executable cache crosses jobs or workflow runs. The
audit step runs after a dependency-policy failure unless the job is cancelled,
so both findings remain visible.

The quality job finds repo-owned `test_*.py` files in deterministic path order
without requiring package directories. It prunes `.git`, `.bench`, and `target`
so checkout metadata, benchmark scratch space, and build output cannot add tests
to the gate.

## Alternatives considered

- Floating action major-version tags were rejected because tags can move after
  review.
- Caching compiled security binaries across runs was rejected for now because
  executing restored tools would require an independently verified content
  digest and carefully scoped cache trust. A single isolated job removes repeated
  setup without adding that trust boundary.
- Keeping one job per security tool was safe but repeated runner, registry, and
  compilation setup.
- Allowing any installed stable compiler was rejected because it would stop
  testing the declared Rust 1.94.1 contract.

## Consequences and verification

GitHub-hosted runners must support Node 24. Updating an action requires reviewing
the upstream release provenance and runtime manifest, then replacing the full SHA
and this table together.

Workflow changes are checked with a YAML parser. Local verification records the
effective Rust and Cargo versions, runs deterministic Python discovery, and runs
the required formatting, Clippy, workspace-test, and doc-test gates. Remote CI
for the exact commit remains required before the milestone or feature is called
complete.
