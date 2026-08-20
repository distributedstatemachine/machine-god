# Architecture

`machine-god-core` contains provider-neutral contracts and orchestration without
ambient operating-system authority. `machine-god-native` provides explicit native
capabilities. `machine-god-cli` composes them. `machine-god-testkit` provides
deterministic test doubles.

Public contracts and lifecycle diagrams will be added with milestone 02. Public
interfaces must keep network, storage, tool, permission, and event delivery behind
object-safe traits.

