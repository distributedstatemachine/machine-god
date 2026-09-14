# Modern ACP validation and review history

This file retains historical evidence, not live delivery status. The
[implementation plan](../implementation-plan.md) owns the current phase and gates.

## Candidate 53096f44: local validation rejection

Candidate: `53096f441f1b927e9494a91434b0b06a95975d2d`.
Base: `658f3366258cf1207904f9c2a274f32db2bb981b`.
No adversarial product-review verdict or delivery acceptance follows this entry.

The Rust 1.94.1 macOS gate passed formatting, warnings-denied Clippy, fresh
release helper construction, focused skills and ACP composition tests, serial
workspace tests, doctests, release smoke, FreeBSD/WASI compilation checks,
pinned-source drift checks, documentation policy, dependency policy and audit.
The release helper SHA-256 was
`f9c4f3ddf99f64b6c49074b78b65b569c72607d1f38aa9c59437676179da3c8e`.
A shell-wrapper failure after workspace/doctest success was traced to an RVM
`cd` hook under `set -u`; remaining checks passed using `builtin cd` without
changing the candidate, helper or test deadlines.

Linux used UID/GID 10001, no DAC-bypass capabilities, a private home, valid
passwd entry, reaping init, bash, zsh, tmux and Rust 1.94.1. Formatting, Clippy,
fresh release build, 54 focused skills tests and the composed ACP round trip
passed. The helper SHA-256 was
`3b936bdc20c17c67d69cbbae99ce84ce0449896e86da1de100461cff546c3e35`.
Default-concurrency workspace execution stopped in CLI units: 532 passed,
one failed and six existing helper fixtures were ignored. The failing
`mcp_commands_cancel_and_session_switch_record_observations_without_replay`
test hit its unchanged ten-second output deadline with an empty retained
transcript. The original failure did not identify the expected output or caller.

One unchanged focused retry and eleven unchanged full CLI unit runs passed.
These retries are diagnostic observations, not a source fix or complete gate.
Read-only tracing established that the shared session picker could silently
discard Enter received after a selectable frame was rendered but before its
flush acknowledgement. That code predates this branch. It is a concrete defect,
but neither the source trace nor the passing retries prove it caused the
recorded timeout. Input received before any selectable frame exists must still
be rejected rather than transferred to a later frame.

The separate unchanged Linux Python suite ran 269 tests in 129.293 seconds.
Its fresh-target release cleanup probe passed with the original 600-second
build and ten-second execution deadlines. The suite nevertheless failed because
Git 2.39.5 lacked `--no-lazy-fetch`, and an equal-length benchmark fixture write
could retain both its original modification and change timestamps. Instrumented
reproduction observed identical before/after timestamps on three of five runs.
This is not a successful complete Python gate. Earlier macOS cold-probe evidence
remains distinct: parent `658f3366` passed in 586.253 seconds, while candidate
`2e8283e3` exceeded the unchanged 600-second build deadline.

Raw macOS evidence is retained under `target/agent-gates/acp-connection.aN177x`;
Linux evidence is under `/cache/target/acp-53096f44.u1HjzP` in the owned validation
cache. The actual executable ACP tests cover process framing/lifecycle; the
successful composed network round trip uses the real host acquisition and stdio
owners with explicitly injected HTTP fixtures, not a live Gateway endpoint.
