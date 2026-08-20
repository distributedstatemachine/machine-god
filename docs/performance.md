# Performance

Performance comparisons must build both projects in release modes on identical
hardware, retain raw samples, warm up before at least 30 measured runs, and report
median, p95, confidence intervals, RSS, and binary sections.

Initial workloads cover startup, help/status/session commands, session replay,
file indexing, deterministic streaming, parallel read-only tools, cancellation,
shutdown, RSS, and binary size. Model inference time is excluded.

