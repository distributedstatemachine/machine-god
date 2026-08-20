# Milestone 01 bootstrap review 01

Reviewed commit: `9b72d92ad608e53443d88f9ef84f671672d26c4e`.

Three fresh agents reviewed correctness/API, security/abuse, and
performance/CI. Confirmed findings and resolutions:

- Missing exact-toolchain CI and security gates: fixed by pinned-action CI with
  Rust 1.94.1, formatting, Clippy, tests, cross-target checks, cargo-deny, and
  cargo-audit.
- Floating evidence toolchain: repository and CI now pin Rust 1.94.1. The local
  machine's exact toolchain is partially installed, so verification in this
  checkout uses its healthy stable toolchain of the same version.
- Cargo resolver v2: changed to resolver v3.
- Publishing fail-open: all workspace packages inherit `publish = false`.
- Self-fulfilling native compatibility version: replaced with an independently
  maintained constant and a deliberate compatibility test.
- Planned security behavior presented as implemented: relabeled as future
  invariants.
- Benchmark evidence was not executable or retained: added a versioned
  collector, checker, tests, exact-SHA CI artifacts, and explicit non-product
  classification.
- Rereview found incomplete provenance validation: the checker now requires host,
  command, warmup, exact Git SHA, recomputed aggregates, hexadecimal checksum,
  and an exact retained binary hash and size.
- Rereview found an invalid vulnerability-action SHA and an action that fetched an
  unverified scanner: both scanners now install exact locked crates verified by
  the Cargo registry, and checkout credentials are not persisted.
- Cross-target checks now cover every target and feature.

Rejected after remediation:

- Package archives omit root license/notice: crates are deliberately
  `publish = false`; packaging is outside the authorized and supported surface.
