#!/usr/bin/env python3
"""Build and measure machine-god beside the exact pinned fx revision.

Schema 2 contains mixed regression evidence. A workload may pass a canonical
equivalence probe here, but remains claim-ineligible until M07 implements the
complete measurement protocol and enforced thresholds.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import ctypes
import fcntl
import hashlib
import json
import math
import os
import platform
import re
import secrets
import select
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, BinaryIO, Callable, Mapping, Sequence


EXPECTED_RUST_VERSION = "1.94.1"
EXPECTED_ZIG_VERSION = "0.16.0"
TERMINATION_SIGNALS = tuple(
    candidate
    for candidate in (
        getattr(signal, "SIGHUP", None),
        signal.SIGINT,
        signal.SIGTERM,
    )
    if candidate is not None
)
BOOTSTRAP_DESCRIPTION = (
    "Launch each release binary through its current no-network bootstrap path"
)
BOOTSTRAP_REASON = (
    "fx uses its FX_BENCH no-argument fast path while machine-god prints its "
    "bootstrap identity; these samples validate the harness and are not product-equivalent"
)
STATUS_HELP_DESCRIPTION = (
    "Equivalent command-specific status help after executable-brand normalization"
)
STATUS_HELP_REASON = (
    "captured outputs passed the executable-brand-only equivalence probe before measurement"
)
STATUS_JSON_DESCRIPTION = (
    "Equivalent isolated status JSON runtime snapshot with a fixed dummy API key"
)
STATUS_JSON_REASON = (
    "captured authenticated outputs passed the pinned runtime-schema equivalence probe "
    "before measurement; missing-auth help is outside this workload"
)
STATUS_JSON_KEYS = (
    "kind",
    "model",
    "update_channel",
    "build_channel",
    "build_revision",
    "auth",
    "auth_refreshable",
    "permission_mode",
    "sandbox",
    "workspace",
    "history_turns",
    "session_permission_grants",
    "agent_step_limit",
)
EQUIVALENCE_PROBE_KEYS = {
    "method",
    "allowed_substitutions",
    "normalized_sha256",
    "implementations",
}


class HarnessSignal(BaseException):
    def __init__(self, signum: int) -> None:
        self.signum = signum


class HarnessSignalState:
    def __init__(self) -> None:
        self.caught_signal: int | None = None
        self.spawning = False
        self.raised_signal = False

    def handle(self, signum: int, _frame: object) -> None:
        if self.caught_signal is not None:
            return
        self.caught_signal = signum
        if not self.spawning:
            self.raised_signal = True
            raise HarnessSignal(signum)

    def raise_if_caught(self) -> None:
        if self.caught_signal is not None:
            self.raised_signal = True
            raise HarnessSignal(self.caught_signal)


ACTIVE_HARNESS_SIGNAL_STATE: HarnessSignalState | None = None


@contextmanager
def termination_signal_handlers():
    global ACTIVE_HARNESS_SIGNAL_STATE

    if ACTIVE_HARNESS_SIGNAL_STATE is not None:
        raise RuntimeError("benchmark termination handlers are already installed")
    state = HarnessSignalState()
    previous = {signum: signal.getsignal(signum) for signum in TERMINATION_SIGNALS}
    ACTIVE_HARNESS_SIGNAL_STATE = state
    try:
        for signum in TERMINATION_SIGNALS:
            signal.signal(signum, state.handle)
        yield
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)
        ACTIVE_HARNESS_SIGNAL_STATE = None
        if state.caught_signal is not None and not state.raised_signal:
            state.raised_signal = True
            raise HarnessSignal(state.caught_signal)


@contextmanager
def defer_harness_signal_while_spawning():
    state = ACTIVE_HARNESS_SIGNAL_STATE
    if state is None:
        yield
        return
    state.raise_if_caught()
    state.spawning = True
    try:
        yield
    finally:
        state.spawning = False
        state.raise_if_caught()


@contextmanager
def defer_harness_signal_while_cleaning():
    """Latch handled termination until non-throwing cleanup reaches its boundary."""

    state = ACTIVE_HARNESS_SIGNAL_STATE
    if state is None:
        yield
        return
    was_spawning = state.spawning
    state.spawning = True
    try:
        yield
    finally:
        state.spawning = was_spawning


def surface_deferred_harness_signal() -> None:
    """Raise a signal latched by cleanup at its explicit caller boundary."""

    state = ACTIVE_HARNESS_SIGNAL_STATE
    if (
        state is not None
        and state.caught_signal is not None
        and not state.raised_signal
    ):
        state.raise_if_caught()


HEX_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
GIT_VERSION_RE = re.compile(r"^git version [0-9]+(?:\.[0-9]+)*(?: \(Apple Git-[0-9]+\))?$")
RUSTC_VERSION_RE = re.compile(
    rf"^rustc {re.escape(EXPECTED_RUST_VERSION)} \([0-9a-f]{{9}} [0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}\)$"
)
CARGO_VERSION_RE = re.compile(
    rf"^cargo {re.escape(EXPECTED_RUST_VERSION)} \([0-9a-f]{{9}} [0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}\)$"
)
CONTAINMENT_ENVIRONMENT_KEY = "MACHINE_GOD_BENCHMARK_RUN_TOKEN"
DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES = 16 * 1024 * 1024
PROCESS_OUTPUT_READ_BYTES = 64 * 1024
ALLOWED_MACHINE_OUTPUTS = (".bench", "benchmarks/results", "target")
BASE_ENVIRONMENT_KEYS = {
    "HOME",
    "LANG",
    "LC_ALL",
    CONTAINMENT_ENVIRONMENT_KEY,
    "NO_COLOR",
    "PATH",
    "TMPDIR",
}
STATUS_JSON_ENVIRONMENT_KEYS = BASE_ENVIRONMENT_KEYS | {
    "AI_GATEWAY_API_KEY",
    "XDG_CONFIG_HOME",
}
STATUS_JSON_DUMMY_API_KEY = "machine-god-benchmark-placeholder-key"
FX_STATUS_CONFIG = b'{"permission_mode":"ask","max_agent_steps":8}\n'
MACHINE_STATUS_CONFIG = (
    b'{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway",'
    b'"transport":"ai_gateway_http","model":"zai/glm-5.2",'
    b'"credential_source":"environment"}\n'
)
GIT_ENVIRONMENT_KEYS = BASE_ENVIRONMENT_KEYS | {
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_TERMINAL_PROMPT",
}
FX_BUILD_ENVIRONMENT_KEYS = BASE_ENVIRONMENT_KEYS | {
    "ZIG_GLOBAL_CACHE_DIR",
    "ZIG_LOCAL_CACHE_DIR",
}
MACHINE_BUILD_ENVIRONMENT_KEYS = BASE_ENVIRONMENT_KEYS | {
    "CARGO_HOME",
    "CARGO_INCREMENTAL",
    "CARGO_TARGET_DIR",
    "RUSTUP_HOME",
}
TOOL_ENVIRONMENT_KEYS = BASE_ENVIRONMENT_KEYS | {"CARGO_HOME", "RUSTUP_HOME"}
FORBIDDEN_ENVIRONMENT_NAMES = {
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "RUSTFLAGS",
}
EXECUTABLE_IDENTITY_KEYS = {
    "executable",
    "canonical_executable",
    "sha256",
    "bytes",
    "mode",
    "device",
    "inode",
    "mtime_ns",
    "ctime_ns",
    "invocation_mode",
    "invocation_device",
    "invocation_inode",
    "invocation_mtime_ns",
    "invocation_ctime_ns",
    "invocation_link_target",
}
COMMAND_RECORD_KEYS = {
    "command",
    "cwd",
    "environment",
    "timeout_seconds",
    "elapsed_ns",
    "setup_ns",
    "supervision_ns",
    "cleanup_ns",
    "returncode",
    "stdout_sha256",
    "stderr_sha256",
}
BINARY_KEYS = {"path", "bytes", "sha256"}
PINNED_EXECUTABLE_KEYS = {
    "method",
    "sha256",
    "bytes",
    "mode",
    "device",
    "inode",
    "seals",
}
SAMPLE_KEYS = {
    "elapsed_ns",
    "setup_ns",
    "supervision_ns",
    "cleanup_ns",
    "returncode",
}


@dataclass(frozen=True)
class UpstreamLock:
    repository: str
    commit: str
    zig: str


@dataclass(frozen=True)
class ProcessResult:
    returncode: int
    stdout: bytes
    stderr: bytes
    elapsed_ns: int
    setup_ns: int
    supervision_ns: int
    cleanup_ns: int


class ProcessTimeout(RuntimeError):
    """A child process exceeded its declared wall-clock limit."""


class ProcessOutputLimit(RuntimeError):
    """A child process exceeded a bounded stdout or stderr capture."""

    def __init__(
        self,
        stream_name: str,
        limit_bytes: int,
        observed_bytes: int,
        command: Sequence[str],
    ) -> None:
        self.stream_name = stream_name
        self.limit_bytes = limit_bytes
        self.observed_bytes = observed_bytes
        super().__init__(
            f"command {stream_name} exceeded {limit_bytes}-byte output limit: "
            f"{' '.join(command)}"
        )


@dataclass(frozen=True)
class LinuxProcessInfo:
    pid: int
    ppid: int
    state: str
    start_time: int


@dataclass(frozen=True)
class MachineStatusEntry:
    state: str
    path: bytes
    original_path: bytes | None


@dataclass(frozen=True)
class OutputLock:
    path: Path
    descriptor: int
    device: int
    inode: int


def parse_upstream_lock(path: Path) -> UpstreamLock:
    """Parse the deliberately small key=value upstream lock format."""

    values: dict[str, str] = {}
    for line_number, raw_line in enumerate(
        path.read_text(encoding="utf-8").splitlines(), 1
    ):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise ValueError(f"{path}:{line_number}: expected key=value")
        key, value = (part.strip() for part in line.split("=", 1))
        if not key or not value:
            raise ValueError(f"{path}:{line_number}: key and value must be non-empty")
        if key in values:
            raise ValueError(f"{path}:{line_number}: duplicate key {key!r}")
        values[key] = value

    expected = {"repository", "commit", "zig"}
    missing = sorted(expected - values.keys())
    unknown = sorted(values.keys() - expected)
    if missing:
        raise ValueError(f"{path}: missing keys: {', '.join(missing)}")
    if unknown:
        raise ValueError(f"{path}: unknown keys: {', '.join(unknown)}")
    if not values["repository"].startswith("https://"):
        raise ValueError(f"{path}: repository must be an HTTPS URL")
    if not HEX_SHA_RE.fullmatch(values["commit"]):
        raise ValueError(f"{path}: commit must be a lowercase 40-character Git SHA")
    if values["zig"] != EXPECTED_ZIG_VERSION:
        raise ValueError(
            f"{path}: this harness requires zig={EXPECTED_ZIG_VERSION}, "
            f"found {values['zig']}"
        )
    return UpstreamLock(**values)


def git_prefix(git: str) -> list[str]:
    return [
        git,
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "protocol.file.allow=never",
        "-c",
        "protocol.ext.allow=never",
        "-c",
        "core.attributesFile=/dev/null",
    ]


def command_plan(
    root: Path,
    upstream_dir: Path,
    lock: UpstreamLock,
    *,
    git: str = "git",
    zig: str = "zig",
    cargo: str = "cargo",
) -> dict[str, list[str]]:
    """Return hardened source and exact release build commands."""

    del root  # The machine build runs at root but does not embed it in argv.
    safe_git = git_prefix(git)
    return {
        "clone": [
            *safe_git,
            "clone",
            "--config",
            "core.hooksPath=/dev/null",
            "--filter=blob:none",
            "--no-checkout",
            lock.repository,
            str(upstream_dir),
        ],
        "fetch": [
            *safe_git,
            "-C",
            str(upstream_dir),
            "fetch",
            "--depth",
            "1",
            "origin",
            lock.commit,
        ],
        "checkout": [
            *safe_git,
            "-C",
            str(upstream_dir),
            "checkout",
            "--detach",
            lock.commit,
        ],
        "fx_build": [zig, "build", "-Doptimize=ReleaseSafe"],
        "machine_god_build": [
            cargo,
            f"+{EXPECTED_RUST_VERSION}",
            "build",
            "--locked",
            "--release",
            "-p",
            "machine-god-cli",
        ],
    }


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def normalize_executable_branding(value: str) -> str:
    """Normalize only standalone fx/machine-god executable branding."""

    return re.sub(
        r"(?<![A-Za-z0-9_])(?:machine-god|fx)(?![A-Za-z0-9_])",
        "<executable>",
        value,
        flags=re.IGNORECASE,
    )


def reject_duplicate_json_members(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate status JSON member: {key}")
        result[key] = value
    return result


def normalize_status_json(
    output: bytes, workspace: Path, expected_build_revision: str
) -> bytes:
    """Validate and normalize the pinned fx status runtime schema."""

    if not output.endswith(b"\n") or output[:-1].endswith(b"\n"):
        raise RuntimeError("status JSON probe must emit exactly one newline-terminated value")
    try:
        parsed = json.loads(
            output[:-1].decode("utf-8", errors="strict"),
            object_pairs_hook=reject_duplicate_json_members,
            parse_constant=lambda value: (_ for _ in ()).throw(
                ValueError(f"non-finite status JSON number: {value}")
            ),
        )
    except (UnicodeError, ValueError, json.JSONDecodeError) as error:
        raise RuntimeError(f"status JSON probe emitted invalid JSON: {error}") from None
    if not isinstance(parsed, dict) or tuple(parsed) != STATUS_JSON_KEYS:
        raise RuntimeError(
            "status JSON probe did not emit the exact pinned runtime schema and key order"
        )
    if parsed["kind"] != "status":
        raise RuntimeError("status JSON probe kind is not status")
    for key in (
        "model",
        "update_channel",
        "build_channel",
        "build_revision",
        "auth",
        "permission_mode",
        "sandbox",
        "workspace",
    ):
        if not isinstance(parsed[key], str):
            raise RuntimeError(f"status JSON probe {key} must be a string")
    if not isinstance(parsed["auth_refreshable"], bool):
        raise RuntimeError("status JSON probe auth_refreshable must be a boolean")
    if parsed["auth"] != "AI_GATEWAY_API_KEY" or parsed["auth_refreshable"] is not False:
        raise RuntimeError("status JSON probe did not use the fixed non-refreshable API-key auth")
    for key in ("history_turns", "session_permission_grants", "agent_step_limit"):
        if (
            not isinstance(parsed[key], int)
            or isinstance(parsed[key], bool)
            or parsed[key] < 0
        ):
            raise RuntimeError(f"status JSON probe {key} must be a nonnegative integer")
    canonical = (
        json.dumps(parsed, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
        + b"\n"
    )
    if output != canonical:
        raise RuntimeError("status JSON probe output is not compact canonical JSON")
    try:
        canonical_workspace = str(workspace.resolve(strict=True))
    except (OSError, RuntimeError, ValueError):
        raise RuntimeError("status JSON probe workspace root is unavailable") from None
    if parsed["workspace"] != canonical_workspace:
        raise RuntimeError("status JSON probe workspace is not the exact canonical root")
    if not (
        expected_build_revision == ""
        or re.fullmatch(r"[0-9a-f]{12}", expected_build_revision)
    ):
        raise RuntimeError("status JSON probe expected build revision is invalid")
    if parsed["build_revision"] != expected_build_revision:
        raise RuntimeError("status JSON probe build revision is not bound to its build")

    parsed["build_revision"] = "<build-provenance>"
    parsed["workspace"] = "<workspace>"
    return json.dumps(parsed, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def normalize_status_help(output: bytes) -> bytes:
    try:
        text = output.decode("utf-8", errors="strict")
    except UnicodeError:
        raise RuntimeError("status help probe emitted non-UTF-8 output") from None
    if not text.endswith("\n") or not text.strip():
        raise RuntimeError("status help probe must emit nonempty newline-terminated help")
    return normalize_executable_branding(text).encode("utf-8")


def stat_identity(metadata: os.stat_result) -> tuple[int, int, int, int, int, int]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def sha256_file(path: Path) -> str:
    invocation = path.absolute()
    canonical = invocation.resolve(strict=True)
    target_before = canonical.lstat()
    open_flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
    open_flags |= getattr(os, "O_CLOEXEC", 0)
    open_flags |= getattr(os, "O_NOFOLLOW", 0)
    open_flags |= getattr(os, "O_NONBLOCK", 0)
    descriptor = os.open(canonical, open_flags)
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise ValueError(f"file is not regular: {canonical}")
        if stat_identity(target_before) != stat_identity(before):
            raise ValueError(f"file path changed before hashing: {canonical}")
        with os.fdopen(descriptor, "rb", buffering=0, closefd=False) as source:
            checksum = bounded_sha256_file(source, before.st_size)
        after = os.fstat(descriptor)
        canonical_after = invocation.resolve(strict=True)
        if canonical_after != canonical:
            raise ValueError(f"file resolution changed while hashed: {invocation}")
        target_after = canonical_after.lstat()
        if stat_identity(target_before) != stat_identity(target_after):
            raise ValueError(f"file path changed while hashed: {canonical}")
        if stat_identity(before) != stat_identity(after):
            raise ValueError(f"file changed while hashed: {canonical}")
        return checksum
    finally:
        os.close(descriptor)


def bounded_sha256_file(source: BinaryIO, expected_bytes: int) -> str:
    """Hash exactly the declared bytes and verify that EOF immediately follows."""

    digest = hashlib.sha256()
    remaining = expected_bytes
    while remaining:
        chunk = source.read(min(remaining, 1024 * 1024))
        if not chunk:
            raise ValueError("supplied binary became shorter during inspection")
        digest.update(chunk)
        remaining -= len(chunk)
    if source.read(1):
        raise ValueError("supplied binary became longer during inspection")
    return digest.hexdigest()


def executable_identity(path: Path) -> dict[str, object]:
    """Bind an invocation path and the canonical executable it dispatches to."""

    invocation = path.absolute()
    invocation_before = invocation.lstat()
    link_target = os.readlink(invocation) if stat.S_ISLNK(invocation_before.st_mode) else ""
    canonical = invocation.resolve(strict=True)
    target_before = canonical.lstat()
    open_flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
    open_flags |= getattr(os, "O_CLOEXEC", 0)
    open_flags |= getattr(os, "O_NOFOLLOW", 0)
    open_flags |= getattr(os, "O_NONBLOCK", 0)
    descriptor = os.open(canonical, open_flags)
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or not before.st_mode & 0o111:
            raise RuntimeError(f"tool is not a regular executable file: {canonical}")
        target_metadata_before = (
            target_before.st_dev,
            target_before.st_ino,
            target_before.st_mode,
            target_before.st_size,
            target_before.st_mtime_ns,
            target_before.st_ctime_ns,
        )
        metadata_before = (
            before.st_dev,
            before.st_ino,
            before.st_mode,
            before.st_size,
            before.st_mtime_ns,
            before.st_ctime_ns,
        )
        if target_metadata_before != metadata_before:
            raise RuntimeError(f"tool target path changed before inspection: {canonical}")
        try:
            with os.fdopen(descriptor, "rb", buffering=0, closefd=False) as source:
                checksum = bounded_sha256_file(source, before.st_size)
        except ValueError as error:
            raise RuntimeError(
                f"tool changed while its identity was read: {canonical}: {error}"
            ) from None
        after = os.fstat(descriptor)
        invocation_after = invocation.lstat()
        invocation_link_target_after = (
            os.readlink(invocation) if stat.S_ISLNK(invocation_after.st_mode) else ""
        )
        canonical_after = invocation.resolve(strict=True)
        target_after = canonical_after.lstat()
    finally:
        try:
            os.close(descriptor)
        except OSError:
            raise RuntimeError(f"failed to close tool after inspection: {canonical}") from None
    invocation_metadata_before = (
        invocation_before.st_dev,
        invocation_before.st_ino,
        invocation_before.st_mode,
        invocation_before.st_mtime_ns,
        invocation_before.st_ctime_ns,
        link_target,
    )
    invocation_metadata_after = (
        invocation_after.st_dev,
        invocation_after.st_ino,
        invocation_after.st_mode,
        invocation_after.st_mtime_ns,
        invocation_after.st_ctime_ns,
        invocation_link_target_after,
    )
    if invocation_metadata_before != invocation_metadata_after:
        raise RuntimeError(f"tool invocation path changed while inspected: {invocation}")
    if canonical_after != canonical:
        raise RuntimeError(f"tool resolution changed while inspected: {invocation}")
    target_metadata_after = (
        target_after.st_dev,
        target_after.st_ino,
        target_after.st_mode,
        target_after.st_size,
        target_after.st_mtime_ns,
        target_after.st_ctime_ns,
    )
    metadata_after = (
        after.st_dev,
        after.st_ino,
        after.st_mode,
        after.st_size,
        after.st_mtime_ns,
        after.st_ctime_ns,
    )
    if target_metadata_before != target_metadata_after:
        raise RuntimeError(f"tool target path changed while inspected: {canonical}")
    if metadata_before != metadata_after:
        raise RuntimeError(f"tool changed while its identity was read: {canonical}")
    return {
        "executable": str(invocation),
        "canonical_executable": str(canonical),
        "sha256": checksum,
        "bytes": before.st_size,
        "mode": stat.S_IMODE(before.st_mode),
        "device": before.st_dev,
        "inode": before.st_ino,
        "mtime_ns": before.st_mtime_ns,
        "ctime_ns": before.st_ctime_ns,
        "invocation_mode": stat.S_IMODE(invocation_before.st_mode),
        "invocation_device": invocation_before.st_dev,
        "invocation_inode": invocation_before.st_ino,
        "invocation_mtime_ns": invocation_before.st_mtime_ns,
        "invocation_ctime_ns": invocation_before.st_ctime_ns,
        "invocation_link_target": link_target,
    }


def verify_executable_identity(identity: Mapping[str, object]) -> None:
    executable = Path(require_text(identity.get("executable"), "tool.executable"))
    try:
        actual = executable_identity(executable)
    except OSError as error:
        raise RuntimeError(f"verified tool is unavailable: {executable}: {error}") from error
    for field in (
        "executable",
        "canonical_executable",
        "sha256",
        "bytes",
        "mode",
        "device",
        "inode",
        "mtime_ns",
        "ctime_ns",
        "invocation_mode",
        "invocation_device",
        "invocation_inode",
        "invocation_mtime_ns",
        "invocation_ctime_ns",
        "invocation_link_target",
    ):
        if actual[field] != identity.get(field):
            raise RuntimeError(f"verified tool identity changed: {executable} ({field})")


def validate_executable_identity_record(
    value: object, field: str
) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{field} must be an object")
    if set(value) != EXECUTABLE_IDENTITY_KEYS:
        raise ValueError(f"{field} fields are not canonical")
    executable = require_text(value.get("executable"), f"{field}.executable")
    canonical = require_text(
        value.get("canonical_executable"), f"{field}.canonical_executable"
    )
    if not Path(executable).is_absolute() or not Path(canonical).is_absolute():
        raise ValueError(f"{field} paths must be absolute")
    checksum = require_text(value.get("sha256"), f"{field}.sha256")
    if len(checksum) != 64 or any(
        character not in "0123456789abcdef" for character in checksum
    ):
        raise ValueError(f"{field}.sha256 must be a lowercase SHA-256 digest")
    for name in (
        "bytes",
        "mode",
        "device",
        "inode",
        "mtime_ns",
        "ctime_ns",
        "invocation_mode",
        "invocation_device",
        "invocation_inode",
        "invocation_mtime_ns",
        "invocation_ctime_ns",
    ):
        if not is_integer(value.get(name)) or value[name] < 0:
            raise ValueError(f"{field}.{name} must be a nonnegative integer")
    if value["bytes"] <= 0 or value["mode"] & 0o111 == 0:
        raise ValueError(f"{field} must identify a non-empty executable")
    if not isinstance(value.get("invocation_link_target"), str):
        raise ValueError(f"{field}.invocation_link_target must be a string")
    return value


def require_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field} must be a non-empty string")
    return value


def validate_generated_at_utc(value: object) -> None:
    timestamp = require_text(value, "generated_at_utc")
    try:
        parsed = datetime.fromisoformat(timestamp)
    except ValueError:
        raise ValueError("generated_at_utc must be a canonical UTC timestamp") from None
    canonical = parsed.isoformat().replace("+00:00", "Z")
    if parsed.tzinfo != timezone.utc or timestamp != canonical:
        raise ValueError("generated_at_utc must be a canonical UTC timestamp")


def is_integer(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def is_positive_number(value: object) -> bool:
    if isinstance(value, bool):
        return False
    if isinstance(value, int):
        return value > 0
    return isinstance(value, float) and math.isfinite(value) and value > 0


def require_command(value: object, field: str) -> list[str]:
    if not isinstance(value, list) or not value:
        raise ValueError(f"{field} must be a non-empty list")
    for index, argument in enumerate(value):
        require_text(argument, f"{field}[{index}]")
    return value


def require_environment(
    value: object, field: str, expected_keys: set[str]
) -> dict[str, str]:
    if not isinstance(value, dict):
        raise ValueError(f"{field} must be an object")
    if set(value) != expected_keys:
        raise ValueError(
            f"{field} keys must be exactly {', '.join(sorted(expected_keys))}"
        )
    for name, item in value.items():
        if not isinstance(name, str) or not isinstance(item, str):
            raise ValueError(f"{field} names and values must be strings")
        if name in FORBIDDEN_ENVIRONMENT_NAMES or name.startswith("CARGO_PROFILE_"):
            raise ValueError(f"{field} contains unsafe ambient override {name}")
    if value["LANG"] != "C" or value["LC_ALL"] != "C" or value["NO_COLOR"] != "1":
        raise ValueError(f"{field} must pin locale and color behavior")
    token = value[CONTAINMENT_ENVIRONMENT_KEY]
    if len(token) != 32 or any(character not in "0123456789abcdef" for character in token):
        raise ValueError(f"{field} has an invalid process-containment token")
    return value


def percentile_95(samples: Sequence[int]) -> int:
    ordered = sorted(samples)
    index = min(len(ordered) - 1, (len(ordered) * 95 + 99) // 100 - 1)
    return ordered[index]


def integer_median(samples: Sequence[int]) -> int:
    ordered = sorted(samples)
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[middle]
    return (ordered[middle - 1] + ordered[middle]) // 2


def validate_binary(binary: object, field: str) -> dict[str, Any]:
    if not isinstance(binary, dict):
        raise ValueError(f"{field} must be an object")
    if set(binary) != BINARY_KEYS:
        raise ValueError(f"{field} fields are not canonical")
    require_text(binary.get("path"), f"{field}.path")
    if not is_integer(binary.get("bytes")) or binary["bytes"] <= 0:
        raise ValueError(f"{field}.bytes must be a positive integer")
    checksum = require_text(binary.get("sha256"), f"{field}.sha256")
    if len(checksum) != 64 or any(
        character not in "0123456789abcdef" for character in checksum
    ):
        raise ValueError(f"{field}.sha256 must be a lowercase SHA-256 digest")
    return binary


def validate_binary_file(binary: Mapping[str, Any], actual: Path, field: str) -> None:
    try:
        expected_path = Path(
            require_text(binary.get("path"), f"{field}.path")
        ).resolve()
        actual_path = actual.resolve()
    except (OSError, RuntimeError, ValueError):
        raise ValueError(f"{field}.path cannot be resolved") from None
    if expected_path != actual_path:
        raise ValueError(f"{field}.path does not match the supplied binary")
    declared_bytes = binary.get("bytes")
    if not is_integer(declared_bytes) or declared_bytes <= 0:
        raise ValueError(f"{field}.bytes must be a positive integer")
    open_flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
    open_flags |= getattr(os, "O_CLOEXEC", 0)
    open_flags |= getattr(os, "O_NOFOLLOW", 0)
    open_flags |= getattr(os, "O_NONBLOCK", 0)
    try:
        descriptor = os.open(actual_path, open_flags)
    except (OSError, RuntimeError, ValueError):
        raise ValueError(f"failed to inspect supplied binary for {field}") from None
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise ValueError(f"supplied binary for {field} is not a regular file")
        execute_bits = stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
        lacks_posix_execute_mode = (
            os.name == "posix" and not before.st_mode & execute_bits
        )
        if lacks_posix_execute_mode or not os.access(actual_path, os.X_OK):
            raise ValueError(f"supplied binary for {field} is not executable")
        if before.st_size != declared_bytes:
            raise ValueError(f"{field}.bytes does not match the supplied binary")
        with os.fdopen(descriptor, "rb", buffering=0, closefd=False) as source:
            checksum = bounded_sha256_file(source, declared_bytes)
        after = os.fstat(descriptor)
        path_after = os.lstat(actual_path)
        before_identity = (
            before.st_dev,
            before.st_ino,
            before.st_mode,
            before.st_size,
            before.st_mtime_ns,
            before.st_ctime_ns,
        )
        after_identity = (
            after.st_dev,
            after.st_ino,
            after.st_mode,
            after.st_size,
            after.st_mtime_ns,
            after.st_ctime_ns,
        )
        path_identity = (
            path_after.st_dev,
            path_after.st_ino,
            path_after.st_mode,
            path_after.st_size,
            path_after.st_mtime_ns,
            path_after.st_ctime_ns,
        )
        if before_identity != path_identity:
            raise ValueError(f"supplied binary path for {field} changed while inspected")
        if before_identity != after_identity:
            raise ValueError(f"supplied binary for {field} changed while inspected")
        if checksum != binary.get("sha256"):
            raise ValueError(f"{field}.sha256 does not match the supplied binary")
    except ValueError:
        raise
    except (OSError, RuntimeError):
        raise ValueError(f"failed to inspect supplied binary for {field}") from None
    finally:
        try:
            os.close(descriptor)
        except OSError:
            raise ValueError(f"failed to close supplied binary for {field}") from None


def validate_command_record(
    record: object,
    field: str,
    *,
    expected_command: Sequence[str] | None = None,
    expected_environment_keys: set[str],
    expected_timeout: float,
    extra_keys: set[str] | None = None,
) -> dict[str, Any]:
    if not isinstance(record, dict):
        raise ValueError(f"{field} must be an object")
    if set(record) != COMMAND_RECORD_KEYS | (extra_keys or set()):
        raise ValueError(f"{field} fields are not canonical")
    command = require_command(record.get("command"), f"{field}.command")
    if expected_command is not None and command != list(expected_command):
        raise ValueError(f"{field}.command is not the exact expected command")
    require_text(record.get("cwd"), f"{field}.cwd")
    require_environment(
        record.get("environment"), f"{field}.environment", expected_environment_keys
    )
    timeout = record.get("timeout_seconds")
    if not is_positive_number(timeout) or timeout != expected_timeout:
        raise ValueError(f"{field}.timeout_seconds does not match the declared timeout")
    if not is_integer(record.get("elapsed_ns")) or record["elapsed_ns"] <= 0:
        raise ValueError(f"{field}.elapsed_ns must be a positive integer")
    for duration in ("setup_ns", "supervision_ns", "cleanup_ns"):
        if not is_integer(record.get(duration)) or record[duration] < 0:
            raise ValueError(f"{field}.{duration} must be a nonnegative integer")
    if record.get("returncode") != 0 or not is_integer(record.get("returncode")):
        raise ValueError(f"{field}.returncode must be integer zero")
    for stream in ("stdout_sha256", "stderr_sha256"):
        checksum = require_text(record.get(stream), f"{field}.{stream}")
        if len(checksum) != 64 or any(
            character not in "0123456789abcdef" for character in checksum
        ):
            raise ValueError(f"{field}.{stream} must be a lowercase SHA-256 digest")
    return record


def validate_measurement(
    measurement: Mapping[str, Any],
    field: str,
    *,
    expected_command: Sequence[str],
    expected_binary: Mapping[str, Any],
    expected_environment_keys: set[str],
    expected_timeout: float,
) -> None:
    command = require_command(measurement.get("command"), f"{field}.command")
    if command != list(expected_command):
        raise ValueError(f"{field}.command is not bound to the built binary")
    require_text(measurement.get("cwd"), f"{field}.cwd")
    require_environment(
        measurement.get("environment"),
        f"{field}.environment",
        expected_environment_keys,
    )
    timeout = measurement.get("timeout_seconds")
    if not is_positive_number(timeout) or timeout != expected_timeout:
        raise ValueError(f"{field}.timeout_seconds does not match the declared timeout")
    identity = validate_executable_identity_record(
        measurement.get("executable_identity"), f"{field}.executable_identity"
    )
    if (
        identity["executable"] != expected_binary["path"]
        or identity["bytes"] != expected_binary["bytes"]
        or identity["sha256"] != expected_binary["sha256"]
    ):
        raise ValueError(f"{field}.executable_identity does not bind the build binary")
    pinned = measurement.get("pinned_executable")
    if not isinstance(pinned, dict):
        raise ValueError(f"{field}.pinned_executable must be an object")
    if set(pinned) != PINNED_EXECUTABLE_KEYS:
        raise ValueError(f"{field}.pinned_executable fields are not canonical")
    if not isinstance(pinned.get("method"), str) or pinned["method"] not in {
        "linux-sealed-memfd-fexecve",
        "private-copy",
    }:
        raise ValueError(f"{field}.pinned_executable.method is unsupported")
    for name in ("bytes", "mode", "device", "inode", "seals"):
        if not is_integer(pinned.get(name)) or pinned[name] < 0:
            raise ValueError(f"{field}.pinned_executable.{name} must be nonnegative")
    if (
        pinned["bytes"] != expected_binary["bytes"]
        or pinned.get("sha256") != expected_binary["sha256"]
        or pinned["mode"] & 0o111 == 0
        or (
            pinned["method"] == "linux-sealed-memfd-fexecve"
            and pinned["seals"] == 0
        )
    ):
        raise ValueError(f"{field}.pinned_executable does not bind immutable bytes")
    warmup = measurement.get("warmup")
    if not is_integer(warmup) or warmup < 1:
        raise ValueError(f"{field}.warmup must be a positive integer")
    samples = measurement.get("samples")
    if not isinstance(samples, list) or len(samples) < 10:
        raise ValueError(f"{field} needs at least 10 raw samples")
    elapsed: list[int] = []
    for index, sample in enumerate(samples):
        if not isinstance(sample, dict) or set(sample) != SAMPLE_KEYS:
            raise ValueError(f"{field}.samples[{index}] must be an object")
        elapsed_ns = sample.get("elapsed_ns")
        if not is_integer(elapsed_ns) or elapsed_ns <= 0:
            raise ValueError(f"{field}.samples[{index}].elapsed_ns must be positive")
        for duration in ("setup_ns", "supervision_ns", "cleanup_ns"):
            if not is_integer(sample.get(duration)) or sample[duration] < 0:
                raise ValueError(
                    f"{field}.samples[{index}].{duration} must be a nonnegative integer"
                )
        if sample.get("returncode") != 0 or not is_integer(sample.get("returncode")):
            raise ValueError(f"{field}.samples[{index}].returncode must be integer zero")
        elapsed.append(elapsed_ns)
    median_ns = measurement.get("median_ns")
    if not is_integer(median_ns) or median_ns != integer_median(elapsed):
        raise ValueError(f"{field}.median_ns does not match raw samples")
    p95_ns = measurement.get("p95_ns")
    if not is_integer(p95_ns) or p95_ns != percentile_95(elapsed):
        raise ValueError(f"{field}.p95_ns does not match raw samples")


def validate_upstream_evidence(
    data: Mapping[str, Any],
    *,
    expected_lock: UpstreamLock | None = None,
    expected_lock_path: Path | None = None,
    expected_lock_sha256: str | None = None,
    expected_root: Path | None = None,
    expected_runner_class: str | None = None,
    expected_machine_tree: str | None = None,
    expected_machine_manifest_sha256: str | None = None,
    expected_binaries: Mapping[str, Path] | None = None,
) -> None:
    """Validate provenance, probes, and bounded regression workloads."""

    expected_root_keys = {
        "schema_version",
        "classification",
        "claim_eligible",
        "generated_at_utc",
        "runner_class",
        "timeouts_seconds",
        "source",
        "host",
        "tools",
        "tool_environment",
        "builds",
        "environment_policy",
        "workloads",
    }
    if set(data) != expected_root_keys:
        raise ValueError("upstream benchmark evidence fields are not canonical")
    if data.get("schema_version") != 2 or not is_integer(data.get("schema_version")):
        raise ValueError("unsupported upstream benchmark schema")
    if data.get("classification") != "mixed-pinned-comparison-evidence":
        raise ValueError("upstream harness evidence classification is not canonical")
    if data.get("claim_eligible") is not False:
        raise ValueError("the mixed evidence document itself must not be claim eligible")
    validate_generated_at_utc(data.get("generated_at_utc"))
    runner_class = require_text(data.get("runner_class"), "runner_class")
    if expected_runner_class is not None and runner_class != expected_runner_class:
        raise ValueError("evidence runner class does not match the expected runner class")

    timeouts = data.get("timeouts_seconds")
    if not isinstance(timeouts, dict) or set(timeouts) != {"fetch", "build", "sample"}:
        raise ValueError("timeouts_seconds must define fetch, build, and sample")
    for name, value in timeouts.items():
        if not is_positive_number(value):
            raise ValueError(f"timeouts_seconds.{name} must be a positive finite number")

    source = data.get("source")
    if not isinstance(source, dict) or set(source) != {"machine_god", "fx"}:
        raise ValueError("source provenance is missing")
    machine_source = source.get("machine_god")
    fx_source = source.get("fx")
    if not isinstance(machine_source, dict) or not isinstance(fx_source, dict):
        raise ValueError("both source revisions are required")
    if set(machine_source) != {
        "git_sha",
        "dirty",
        "repository_root",
        "allowed_output_directories",
        "materialization",
    }:
        raise ValueError("machine-god source fields are not canonical")
    if set(fx_source) != {
        "repository",
        "locked_commit",
        "verified_commit",
        "lock_path",
        "lock_sha256",
        "fresh_checkout",
        "hooks_disabled",
        "preparation_commands",
    }:
        raise ValueError("fx source fields are not canonical")
    machine_sha = require_text(machine_source.get("git_sha"), "source.machine_god.git_sha")
    if not HEX_SHA_RE.fullmatch(machine_sha):
        raise ValueError("source.machine_god.git_sha must be a lowercase 40-character SHA")
    if machine_source.get("dirty") is not False:
        raise ValueError("machine-god source must be clean")
    if machine_source.get("allowed_output_directories") != list(ALLOWED_MACHINE_OUTPUTS):
        raise ValueError("machine-god cleanliness exceptions are not canonical")

    repository = require_text(fx_source.get("repository"), "source.fx.repository")
    locked_commit = require_text(fx_source.get("locked_commit"), "source.fx.locked_commit")
    verified_commit = require_text(
        fx_source.get("verified_commit"), "source.fx.verified_commit"
    )
    if not HEX_SHA_RE.fullmatch(locked_commit) or verified_commit != locked_commit:
        raise ValueError("the verified fx commit must equal the locked 40-character SHA")
    if expected_lock is not None and (
        repository != expected_lock.repository or locked_commit != expected_lock.commit
    ):
        raise ValueError("evidence does not match the canonical upstream lock")
    if fx_source.get("fresh_checkout") is not True:
        raise ValueError("fx source must come from a fresh checkout")
    if fx_source.get("hooks_disabled") is not True:
        raise ValueError("fx Git hooks must be disabled")
    recorded_lock_path = Path(
        require_text(fx_source.get("lock_path"), "source.fx.lock_path")
    ).resolve()
    if expected_lock_path is not None and recorded_lock_path != expected_lock_path.resolve():
        raise ValueError("evidence does not name the canonical upstream lock path")
    lock_checksum = require_text(fx_source.get("lock_sha256"), "source.fx.lock_sha256")
    if len(lock_checksum) != 64 or any(
        character not in "0123456789abcdef" for character in lock_checksum
    ):
        raise ValueError("source.fx.lock_sha256 must be a lowercase SHA-256 digest")
    if expected_lock_sha256 is not None and lock_checksum != expected_lock_sha256:
        raise ValueError("evidence does not bind the canonical upstream lock bytes")

    host = data.get("host")
    if not isinstance(host, dict) or set(host) != {
        "system",
        "release",
        "machine",
        "python",
        "cpu_count",
        "cpu_model",
        "runner",
    }:
        raise ValueError("host metadata is missing")
    for field in ("system", "release", "machine", "python", "cpu_model"):
        require_text(host.get(field), f"host.{field}")
    if not is_integer(host.get("cpu_count")) or host["cpu_count"] < 1:
        raise ValueError("host.cpu_count must be a positive integer")
    runner = host.get("runner")
    if (
        not isinstance(runner, dict)
        or set(runner)
        != {
            "class",
            "github_actions",
            "image_os",
            "image_version",
            "runner_os",
            "runner_arch",
        }
        or runner.get("class") != runner_class
    ):
        raise ValueError("host.runner.class must bind the evidence runner class")
    for field in ("image_os", "image_version", "runner_os", "runner_arch"):
        require_text(runner.get(field), f"host.runner.{field}")
    if not isinstance(runner.get("github_actions"), bool):
        raise ValueError("host.runner.github_actions must be boolean")

    tools = data.get("tools")
    if not isinstance(tools, dict) or set(tools) != {"git", "zig", "rustc", "cargo"}:
        raise ValueError("tool provenance is missing")
    for name in ("git", "zig", "rustc", "cargo"):
        tool = tools.get(name)
        if not isinstance(tool, dict):
            raise ValueError(f"tools.{name} is missing")
        expected_tool_keys = EXECUTABLE_IDENTITY_KEYS | {"command", "version"}
        if name != "git":
            expected_tool_keys |= {"required_version"}
        if set(tool) != expected_tool_keys:
            raise ValueError(f"tools.{name} fields are not canonical")
        command = require_command(tool.get("command"), f"tools.{name}.command")
        executable = require_text(tool.get("executable"), f"tools.{name}.executable")
        if command[0] != executable:
            raise ValueError(f"tools.{name}.command is not bound to its executable")
        canonical_executable = require_text(
            tool.get("canonical_executable"), f"tools.{name}.canonical_executable"
        )
        if not Path(executable).is_absolute():
            raise ValueError(f"tools.{name}.executable must be an absolute path")
        if (
            not Path(canonical_executable).is_absolute()
            or Path(canonical_executable).resolve() != Path(canonical_executable)
            or Path(executable).resolve() != Path(canonical_executable)
        ):
            raise ValueError(f"tools.{name}.canonical_executable is not canonical")
        checksum = require_text(tool.get("sha256"), f"tools.{name}.sha256")
        if len(checksum) != 64 or any(
            character not in "0123456789abcdef" for character in checksum
        ):
            raise ValueError(f"tools.{name}.sha256 must be a lowercase SHA-256 digest")
        for field in (
            "bytes",
            "mode",
            "device",
            "inode",
            "mtime_ns",
            "ctime_ns",
            "invocation_mode",
            "invocation_device",
            "invocation_inode",
            "invocation_mtime_ns",
            "invocation_ctime_ns",
        ):
            if not is_integer(tool.get(field)) or tool[field] < 0:
                raise ValueError(f"tools.{name}.{field} must be a non-negative integer")
        if not isinstance(tool.get("invocation_link_target"), str):
            raise ValueError(f"tools.{name}.invocation_link_target must be a string")
        if tool["bytes"] <= 0 or tool["mode"] & 0o111 == 0:
            raise ValueError(f"tools.{name} must identify a non-empty executable file")
        require_text(tool.get("version"), f"tools.{name}.version")
    expected_tool_commands = {
        "git": [tools["git"]["executable"], "--version"],
        "zig": [tools["zig"]["executable"], "version"],
        "rustc": [
            tools["rustc"]["executable"],
            f"+{EXPECTED_RUST_VERSION}",
            "--version",
        ],
        "cargo": [
            tools["cargo"]["executable"],
            f"+{EXPECTED_RUST_VERSION}",
            "--version",
        ],
    }
    for name, command in expected_tool_commands.items():
        if tools[name]["command"] != command:
            raise ValueError(f"tools.{name}.command is not the exact version command")
    if tools["zig"].get("required_version") != EXPECTED_ZIG_VERSION:
        raise ValueError("tools.zig.required_version is not pinned to 0.16.0")
    if tools["zig"].get("version") != EXPECTED_ZIG_VERSION:
        raise ValueError("evidence was not built with Zig 0.16.0")
    if expected_lock is not None and tools["zig"]["required_version"] != expected_lock.zig:
        raise ValueError("evidence does not match the canonical upstream lock")
    if not GIT_VERSION_RE.fullmatch(tools["git"]["version"]):
        raise ValueError("evidence has a noncanonical Git version")
    for name, version_pattern in (
        ("rustc", RUSTC_VERSION_RE),
        ("cargo", CARGO_VERSION_RE),
    ):
        if tools[name].get("required_version") != EXPECTED_RUST_VERSION:
            raise ValueError(f"tools.{name}.required_version is not pinned to 1.94.1")
        if not version_pattern.fullmatch(tools[name]["version"]):
            raise ValueError(f"evidence was not built with {name} {EXPECTED_RUST_VERSION}")
    tool_environment = require_environment(
        data.get("tool_environment"), "tool_environment", TOOL_ENVIRONMENT_KEYS
    )
    policy = data.get("environment_policy")
    if (
        not isinstance(policy, dict)
        or set(policy)
        != {"inherits_parent_environment", "allowlisted_environment_only"}
        or policy["inherits_parent_environment"] is not False
        or policy["allowlisted_environment_only"] is not True
    ):
        raise ValueError("environment_policy must forbid ambient inheritance")

    repository_root = Path(
        require_text(machine_source.get("repository_root"), "source.machine_god.repository_root")
    ).resolve()
    if expected_root is not None and repository_root != expected_root.resolve():
        raise ValueError("machine-god repository root is not canonical")
    materialization = machine_source.get("materialization")
    if (
        not isinstance(materialization, dict)
        or set(materialization)
        != {
            "method",
            "source_dir",
            "manifest_path",
            "git_tree",
            "entries",
            "git_entries_sha256",
            "manifest_sha256",
            "source_tree_sha256",
            "listing_command",
        }
        or materialization.get("method") != "git-ls-tree-cat-file"
    ):
        raise ValueError("machine-god source must use canonical Git-object materialization")
    source_dir = Path(
        require_text(
            materialization.get("source_dir"),
            "source.machine_god.materialization.source_dir",
        )
    ).resolve()
    manifest_path = Path(
        require_text(
            materialization.get("manifest_path"),
            "source.machine_god.materialization.manifest_path",
        )
    ).resolve()
    scratch_dir = source_dir.parent
    if source_dir != scratch_dir / "machine-source":
        raise ValueError("machine source path is not derived from the scratch directory")
    if manifest_path != scratch_dir / "machine-source-manifest.json":
        raise ValueError("machine source manifest path is not derived from scratch")
    for field in ("manifest_sha256", "source_tree_sha256"):
        checksum = require_text(
            materialization.get(field), f"source.machine_god.materialization.{field}"
        )
        if len(checksum) != 64 or any(
            character not in "0123456789abcdef" for character in checksum
        ):
            raise ValueError(f"source.machine_god.materialization.{field} is not SHA-256")
    git_tree = require_text(
        materialization.get("git_tree"), "source.machine_god.materialization.git_tree"
    )
    if not HEX_SHA_RE.fullmatch(git_tree):
        raise ValueError("source.machine_god.materialization.git_tree is not a Git tree SHA")
    if expected_machine_tree is not None and git_tree != expected_machine_tree:
        raise ValueError("materialized machine source tree does not match recorded commit")
    if (
        expected_machine_manifest_sha256 is not None
        and materialization.get("git_entries_sha256")
        != expected_machine_manifest_sha256
    ):
        raise ValueError("machine source manifest does not match the recorded Git tree")
    if Path(tool_environment["HOME"]).resolve() != scratch_dir / "home":
        raise ValueError("tool HOME is not derived from the scratch directory")
    if Path(tool_environment["TMPDIR"]).resolve() != scratch_dir / "tmp":
        raise ValueError("tool TMPDIR is not derived from the scratch directory")
    if Path(tool_environment["CARGO_HOME"]).resolve() != scratch_dir / "cargo-home":
        raise ValueError("tool CARGO_HOME is not the isolated scratch cache")
    materialization_command = validate_command_record(
        materialization.get("listing_command"),
        "source.machine_god.materialization.listing_command",
        expected_command=machine_tree_command(tools["git"]["executable"], machine_sha),
        expected_environment_keys=GIT_ENVIRONMENT_KEYS,
        expected_timeout=timeouts["fetch"],
    )
    if Path(materialization_command["cwd"]).resolve() != repository_root:
        raise ValueError("machine source listing command did not run from repository root")
    materialization_environment = materialization_command["environment"]
    if (
        materialization_environment["GIT_CONFIG_GLOBAL"] != "/dev/null"
        or materialization_environment["GIT_CONFIG_NOSYSTEM"] != "1"
        or materialization_environment["GIT_NO_REPLACE_OBJECTS"] != "1"
        or materialization_environment["GIT_TERMINAL_PROMPT"] != "0"
    ):
        raise ValueError("machine source listing Git environment is not isolated")
    for key in BASE_ENVIRONMENT_KEYS:
        if materialization_environment[key] != tool_environment[key]:
            raise ValueError(f"machine source materialization changes canonical {key}")
    entries = validate_source_manifest(
        materialization.get("entries"), "source.machine_god.materialization.entries"
    )
    git_entries_sha256 = require_text(
        materialization.get("git_entries_sha256"),
        "source.machine_god.materialization.git_entries_sha256",
    )
    if git_entries_sha256 != canonical_git_entries_sha256(entries):
        raise ValueError("machine source Git-entry checksum does not match its entries")
    if canonical_manifest_sha256(entries) != materialization.get("manifest_sha256"):
        raise ValueError("machine source manifest checksum does not match its entries")
    if canonical_manifest_sha256(entries) != materialization.get("source_tree_sha256"):
        raise ValueError("machine source tree checksum does not match its entries")

    builds = data.get("builds")
    if not isinstance(builds, list) or len(builds) != 2:
        raise ValueError("exactly two build records are required")
    if [build.get("project") for build in builds if isinstance(build, dict)] != [
        "fx",
        "machine-god",
    ]:
        raise ValueError("build records must be ordered as fx then machine-god")
    fx_build, machine_build = builds
    if fx_build.get("profile") != "ReleaseSafe":
        raise ValueError("fx build profile must be ReleaseSafe")
    if machine_build.get("profile") != "release":
        raise ValueError("machine-god build profile must be release")
    fx_command = [tools["zig"]["executable"], "build", "-Doptimize=ReleaseSafe"]
    machine_command = [
        tools["cargo"]["executable"],
        f"+{EXPECTED_RUST_VERSION}",
        "build",
        "--locked",
        "--release",
        "-p",
        "machine-god-cli",
    ]
    validate_command_record(
        fx_build,
        "builds[0]",
        expected_command=fx_command,
        expected_environment_keys=FX_BUILD_ENVIRONMENT_KEYS,
        expected_timeout=timeouts["build"],
        extra_keys={"project", "profile", "binary"},
    )
    validate_command_record(
        machine_build,
        "builds[1]",
        expected_command=machine_command,
        expected_environment_keys=MACHINE_BUILD_ENVIRONMENT_KEYS,
        expected_timeout=timeouts["build"],
        extra_keys={"project", "profile", "binary"},
    )
    if Path(machine_build["cwd"]).resolve() != source_dir:
        raise ValueError("machine-god build did not use the materialized source tree")
    for key in BASE_ENVIRONMENT_KEYS:
        if fx_build["environment"][key] != tool_environment[key]:
            raise ValueError(f"fx build environment changes canonical {key}")
        if machine_build["environment"][key] != tool_environment[key]:
            raise ValueError(f"machine-god build environment changes canonical {key}")
    if machine_build["environment"]["CARGO_INCREMENTAL"] != "0":
        raise ValueError("machine-god release build must disable incremental compilation")
    if machine_build["environment"]["CARGO_HOME"] != tool_environment["CARGO_HOME"]:
        raise ValueError("machine-god build CARGO_HOME differs from the verified tool environment")
    if machine_build["environment"]["RUSTUP_HOME"] != tool_environment["RUSTUP_HOME"]:
        raise ValueError("machine-god build RUSTUP_HOME differs from the verified tool environment")
    if Path(machine_build["environment"]["CARGO_TARGET_DIR"]).resolve() != (
        scratch_dir / "machine-target"
    ):
        raise ValueError("machine-god target path is not derived from scratch")
    if Path(fx_build["environment"]["ZIG_GLOBAL_CACHE_DIR"]).resolve() != (
        scratch_dir / "zig-global-cache"
    ):
        raise ValueError("fx global cache path is not derived from scratch")
    if Path(fx_build["environment"]["ZIG_LOCAL_CACHE_DIR"]).resolve() != (
        Path(fx_build["cwd"]) / ".zig-cache"
    ).resolve():
        raise ValueError("fx local cache must be isolated inside the fresh checkout")
    fx_binary = validate_binary(fx_build.get("binary"), "builds[0].binary")
    machine_binary = validate_binary(machine_build.get("binary"), "builds[1].binary")
    expected_fx_path = Path(fx_build["cwd"]) / "zig-out/bin/fx"
    expected_machine_path = (
        Path(machine_build["environment"]["CARGO_TARGET_DIR"])
        / "release/machine-god"
    )
    if Path(fx_binary["path"]).resolve() != expected_fx_path.resolve():
        raise ValueError("fx binary path does not match the exact build output")
    if Path(machine_binary["path"]).resolve() != expected_machine_path.resolve():
        raise ValueError("machine-god binary path does not match the exact build output")

    preparation = fx_source.get("preparation_commands")
    if not isinstance(preparation, list) or len(preparation) != 3:
        raise ValueError("fresh fx preparation must retain clone, fetch, and checkout")
    plan = command_plan(
        repository_root,
        Path(fx_build["cwd"]),
        UpstreamLock(repository, locked_commit, EXPECTED_ZIG_VERSION),
        git=tools["git"]["executable"],
        zig=tools["zig"]["executable"],
        cargo=tools["cargo"]["executable"],
    )
    for index, name in enumerate(("clone", "fetch", "checkout")):
        validate_command_record(
            preparation[index],
            f"source.fx.preparation_commands[{index}]",
            expected_command=plan[name],
            expected_environment_keys=GIT_ENVIRONMENT_KEYS,
            expected_timeout=timeouts["fetch"],
        )
        environment = preparation[index]["environment"]
        if (
            environment["GIT_CONFIG_GLOBAL"] != "/dev/null"
            or environment["GIT_CONFIG_NOSYSTEM"] != "1"
            or environment["GIT_NO_REPLACE_OBJECTS"] != "1"
            or environment["GIT_TERMINAL_PROMPT"] != "0"
        ):
            raise ValueError("source preparation Git environment is not isolated")
        for key in BASE_ENVIRONMENT_KEYS:
            if environment[key] != tool_environment[key]:
                raise ValueError(f"source preparation changes canonical {key}")
    if any(Path(record["cwd"]).resolve() != repository_root for record in preparation):
        raise ValueError("source preparation commands must run from machine-god repository root")

    workloads = data.get("workloads")
    if not isinstance(workloads, list) or len(workloads) != 7:
        raise ValueError("the canonical workload inventory is incomplete")
    expected_workload_keys = {
        "id",
        "description",
        "equivalence",
        "claim_eligible",
        "reason",
        "implementations",
    }
    if any(not isinstance(workload, dict) for workload in workloads):
        raise ValueError("workload fields are not canonical")
    expected_ids = [
        "bootstrap-exit",
        "help",
        "status-help",
        "status-json",
        "doctor-json",
        "sessions-json",
        "background-json",
    ]
    if [workload.get("id") for workload in workloads if isinstance(workload, dict)] != expected_ids:
        raise ValueError("workload identifiers or order are not canonical")
    bootstrap = workloads[0]
    if set(bootstrap) != expected_workload_keys:
        raise ValueError("bootstrap-exit fields are not canonical")
    if (
        bootstrap.get("description") != BOOTSTRAP_DESCRIPTION
        or bootstrap.get("reason") != BOOTSTRAP_REASON
    ):
        raise ValueError("bootstrap-exit narrative is not canonical")
    if (
        bootstrap.get("equivalence") != "non-equivalent"
        or bootstrap.get("claim_eligible") is not False
    ):
        raise ValueError("bootstrap-exit must remain non-equivalent and claim-ineligible")
    implementations = bootstrap.get("implementations")
    if not isinstance(implementations, list) or len(implementations) != 2:
        raise ValueError("bootstrap-exit must contain both measurements")
    if [item.get("project") for item in implementations if isinstance(item, dict)] != [
        "fx",
        "machine-god",
    ]:
        raise ValueError("bootstrap measurements must be ordered as fx then machine-god")
    fx_measurement, machine_measurement = implementations
    expected_measurement_keys = {
        "project",
        "status",
        "command",
        "cwd",
        "environment",
        "timeout_seconds",
        "warmup",
        "executable_identity",
        "pinned_executable",
        "samples",
        "median_ns",
        "p95_ns",
    }
    if (
        not isinstance(fx_measurement, dict)
        or not isinstance(machine_measurement, dict)
        or set(fx_measurement) != expected_measurement_keys
        or set(machine_measurement) != expected_measurement_keys
    ):
        raise ValueError("bootstrap measurement fields are not canonical")
    if fx_measurement.get("status") != "measured" or machine_measurement.get("status") != "measured":
        raise ValueError("both bootstrap implementations must be measured")
    validate_measurement(
        fx_measurement,
        "workloads[0].implementations[0]",
        expected_command=[fx_binary["path"]],
        expected_binary=fx_binary,
        expected_environment_keys=BASE_ENVIRONMENT_KEYS | {"FX_BENCH"},
        expected_timeout=timeouts["sample"],
    )
    validate_measurement(
        machine_measurement,
        "workloads[0].implementations[1]",
        expected_command=[machine_binary["path"]],
        expected_binary=machine_binary,
        expected_environment_keys=BASE_ENVIRONMENT_KEYS,
        expected_timeout=timeouts["sample"],
    )
    if fx_measurement["environment"]["FX_BENCH"] != "1":
        raise ValueError("fx bootstrap measurement must pin FX_BENCH=1")
    for measurement in (fx_measurement, machine_measurement):
        for key in BASE_ENVIRONMENT_KEYS:
            if measurement["environment"][key] != tool_environment[key]:
                raise ValueError(f"bootstrap measurement changes canonical {key}")
    if (
        Path(fx_measurement["cwd"]).resolve() != source_dir
        or Path(machine_measurement["cwd"]).resolve() != source_dir
    ):
        raise ValueError("bootstrap measurements must run from materialized machine-god source")

    implemented_commands = {
        "help": (
            [fx_binary["path"], "help"],
            [machine_binary["path"], "help"],
        ),
        "doctor-json": (
            [fx_binary["path"], "doctor", "--json"],
            [machine_binary["path"], "doctor", "--json"],
        ),
        "sessions-json": (
            [fx_binary["path"], "sessions", "--json"],
            [machine_binary["path"], "sessions", "--json"],
        ),
        "background-json": (
            [fx_binary["path"], "background", "--json"],
            [machine_binary["path"], "background", "--json"],
        ),
    }
    unavailable = unavailable_workloads(
        Path(fx_binary["path"]), Path(machine_binary["path"])
    )
    if workloads[1] != unavailable[0] or workloads[4:] != unavailable[1:]:
        raise ValueError("non-equivalent workload inventory and narratives are not canonical")
    for index in (1, 4, 5, 6):
        workload = workloads[index]
        field = f"workloads[{index}]"
        if set(workload) != expected_workload_keys:
            raise ValueError(f"{field} fields are not canonical")
        if (
            workload.get("equivalence") != "non-equivalent"
            or workload.get("claim_eligible") is not False
        ):
            raise ValueError(f"{field} must remain non-equivalent and claim-ineligible")
        require_text(workload.get("description"), f"{field}.description")
        require_text(workload.get("reason"), f"{field}.reason")
        items = workload.get("implementations")
        if not isinstance(items, list) or len(items) != 2:
            raise ValueError(f"{field} must describe fx and machine-god")
        fx_item, machine_item = items
        if not isinstance(fx_item, dict) or not isinstance(machine_item, dict):
            raise ValueError(f"{field} implementations must be objects")
        fx_command, machine_command = implemented_commands[workload["id"]]
        if (
            fx_item.get("project") != "fx"
            or fx_item.get("status") != "not-measured"
            or require_command(fx_item.get("command"), f"{field}.fx.command")
            != fx_command
        ):
            raise ValueError(f"{field} fx command is not canonical")
        if (
            machine_item.get("project") != "machine-god"
            or machine_item.get("status") != "not-measured"
            or require_command(
                machine_item.get("command"), f"{field}.machine_god.command"
            )
            != machine_command
        ):
            raise ValueError(f"{field} machine-god command is not canonical")
        expected_item_keys = {"project", "status", "command", "reason"}
        if (
            set(fx_item) != expected_item_keys
            or set(machine_item) != expected_item_keys
        ):
            raise ValueError(f"{field} must contain commands but no measurement results")
        require_text(fx_item.get("reason"), f"{field}.fx.reason")
        require_text(machine_item.get("reason"), f"{field}.machine_god.reason")

    comparison_root = source_dir.parent / "comparison"
    equivalent_specs = (
        (
            2,
            "status-help",
            STATUS_HELP_DESCRIPTION,
            STATUS_HELP_REASON,
            "status-help-executable-brand-v1",
            ["executable-branding"],
            [fx_binary["path"], "status", "--help"],
            [machine_binary["path"], "status", "--help"],
        ),
        (
            3,
            "status-json",
            STATUS_JSON_DESCRIPTION,
            STATUS_JSON_REASON,
            "status-json-runtime-schema-v1",
            ["build-provenance", "isolated-workspace-root"],
            [fx_binary["path"], "status", "--json"],
            [machine_binary["path"], "status", "--json"],
        ),
    )
    for (
        index,
        identifier,
        description,
        reason,
        method,
        substitutions,
        fx_command,
        machine_command,
    ) in equivalent_specs:
        workload = workloads[index]
        field = f"workloads[{index}]"
        if set(workload) != expected_workload_keys | {"equivalence_probe"}:
            raise ValueError(f"{field} fields are not canonical")
        if (
            workload.get("id") != identifier
            or workload.get("description") != description
            or workload.get("reason") != reason
            or workload.get("equivalence") != "equivalent"
            or workload.get("claim_eligible") is not False
        ):
            raise ValueError(f"{field} equivalence declaration is not canonical")
        probe = workload.get("equivalence_probe")
        expected_probe_keys = EQUIVALENCE_PROBE_KEYS | (
            {"fixture_sha256"} if identifier == "status-json" else set()
        )
        if not isinstance(probe, dict) or set(probe) != expected_probe_keys:
            raise ValueError(f"{field}.equivalence_probe fields are not canonical")
        if (
            probe.get("method") != method
            or probe.get("allowed_substitutions") != substitutions
        ):
            raise ValueError(f"{field}.equivalence_probe normalization is not canonical")
        normalized_sha = probe.get("normalized_sha256")
        if (
            not isinstance(normalized_sha, str)
            or len(normalized_sha) != 64
            or any(character not in "0123456789abcdef" for character in normalized_sha)
        ):
            raise ValueError(f"{field}.equivalence_probe normalized output is invalid")
        probe_items = probe.get("implementations")
        measurements = workload.get("implementations")
        if not isinstance(probe_items, list) or len(probe_items) != 2:
            raise ValueError(f"{field}.equivalence_probe must contain both implementations")
        if not isinstance(measurements, list) or len(measurements) != 2:
            raise ValueError(f"{field} must contain both measurements")
        if identifier == "status-json":
            if probe.get("fixture_sha256") != {
                "fx_settings": sha256_bytes(FX_STATUS_CONFIG),
                "machine_god_config": sha256_bytes(MACHINE_STATUS_CONFIG),
            }:
                raise ValueError(f"{field}.equivalence_probe fixtures are not canonical")
        for project_index, (project, command, binary) in enumerate(
            (
                ("fx", fx_command, fx_binary),
                ("machine-god", machine_command, machine_binary),
            )
        ):
            expected_root = comparison_root / identifier / project
            expected_cwd = expected_root / "workspace"
            environment_keys = (
                STATUS_JSON_ENVIRONMENT_KEYS
                if identifier == "status-json"
                else BASE_ENVIRONMENT_KEYS
            )
            expected_environment = {
                **{key: tool_environment[key] for key in BASE_ENVIRONMENT_KEYS},
                "HOME": str(
                    expected_cwd / "home"
                    if identifier == "status-json"
                    else expected_root / "home"
                ),
                "TMPDIR": str(
                    expected_cwd / "tmp"
                    if identifier == "status-json"
                    else expected_root / "tmp"
                ),
            }
            if identifier == "status-json":
                expected_environment.update(
                    {
                        "AI_GATEWAY_API_KEY": STATUS_JSON_DUMMY_API_KEY,
                        "XDG_CONFIG_HOME": str(expected_cwd / "xdg-config"),
                    }
                )
            probe_item = probe_items[project_index]
            if (
                not isinstance(probe_item, dict)
                or set(probe_item)
                != {"project", "stdout_sha256", "normalized_sha256"}
                or probe_item.get("project") != project
                or probe_item.get("normalized_sha256") != normalized_sha
                or not isinstance(probe_item.get("stdout_sha256"), str)
                or len(probe_item["stdout_sha256"]) != 64
                or any(
                    character not in "0123456789abcdef"
                    for character in probe_item["stdout_sha256"]
                )
            ):
                raise ValueError(f"{field} equivalence probe result is not canonical")
            measurement = measurements[project_index]
            if not isinstance(measurement, dict) or set(measurement) != expected_measurement_keys:
                raise ValueError(f"{field} measurement fields are not canonical")
            if measurement.get("project") != project or measurement.get("status") != "measured":
                raise ValueError(f"{field} both equivalent implementations must be measured")
            validate_measurement(
                measurement,
                f"{field}.implementations[{project_index}]",
                expected_command=command,
                expected_binary=binary,
                expected_environment_keys=environment_keys,
                expected_timeout=timeouts["sample"],
            )
            if (
                Path(measurement["cwd"]).resolve() != expected_cwd
                or measurement["environment"] != expected_environment
            ):
                raise ValueError(f"{field} measurement does not reuse its isolated probe context")

    if expected_binaries is not None:
        if set(expected_binaries) != {"fx", "machine-god"}:
            raise ValueError("both actual binaries are required")
        validate_binary_file(fx_binary, expected_binaries["fx"], "builds[0].binary")
        validate_binary_file(
            machine_binary,
            expected_binaries["machine-god"],
            "builds[1].binary",
        )
        for name, project_index, actual_path in (
            ("fx", 0, expected_binaries["fx"]),
            ("machine-god", 1, expected_binaries["machine-god"]),
        ):
            try:
                actual_identity = executable_identity(actual_path)
            except (OSError, RuntimeError):
                raise ValueError(f"{name} executable identity is unreadable") from None
            for workload_index in (0, 2, 3):
                measurement = workloads[workload_index]["implementations"][project_index]
                if measurement["executable_identity"] != actual_identity:
                    raise ValueError(
                        f"{name} measured executable identity does not match the supplied binary"
                    )
        status_fixture_files: dict[Path, bytes] = {}
        for project in ("fx", "machine-god"):
            workspace = comparison_root / "status-json" / project / "workspace"
            status_fixture_files[workspace / "home" / ".fx" / "settings.json"] = FX_STATUS_CONFIG
            status_fixture_files[
                workspace / "xdg-config" / "machine-god" / "config.json"
            ] = MACHINE_STATUS_CONFIG
        try:
            verify_status_json_fixtures(status_fixture_files)
        except RuntimeError as error:
            raise ValueError(str(error)) from error
        for tool in tools.values():
            try:
                verify_executable_identity(tool)
            except RuntimeError as error:
                raise ValueError(str(error)) from error
        if (
            not manifest_path.is_file()
            or sha256_file(manifest_path) != materialization["manifest_sha256"]
        ):
            raise ValueError("materialized machine source manifest does not match evidence")
        try:
            manifest_entries = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise ValueError("materialized machine source manifest is unreadable") from error
        if manifest_entries != entries:
            raise ValueError("materialized machine source manifest entries do not match evidence")
        if not source_dir.is_dir():
            raise ValueError("materialized machine source directory is missing")
        try:
            source_digest = verify_materialized_source(source_dir, entries)
        except RuntimeError as error:
            raise ValueError(str(error)) from error
        if source_digest != materialization["source_tree_sha256"]:
            raise ValueError("materialized machine source tree does not match evidence")


_LINUX_PREFLIGHT_COMPLETE = False
_LINUX_PREFLIGHT_LOCK = threading.Lock()
MAX_LINUX_PROCESS_TABLE_ENTRIES = 65_536
MAX_LINUX_CONTAINED_PROCESSES = 4_096


class LinuxProcessTableLimit(RuntimeError):
    def __init__(self, overflow_pid: int) -> None:
        self.overflow_pid = overflow_pid
        super().__init__("Linux /proc process table exceeded the containment entry limit")


class LinuxContainmentProcessLimit(RuntimeError):
    def __init__(
        self,
        discovered: Sequence[LinuxProcessInfo],
        overflow: LinuxProcessInfo,
    ) -> None:
        self.discovered = tuple(discovered)
        self.overflow = overflow
        super().__init__(
            "Linux descendant set exceeded the containment process limit"
        )


def linux_process_info(pid: int) -> LinuxProcessInfo:
    """Read one Linux process identity without scanning the process table."""

    entry = Path("/proc") / str(pid)
    try:
        if entry.stat().st_uid != os.getuid():
            raise RuntimeError(f"Linux process PID {pid} is not owned by this user")
        contents = (entry / "stat").read_text(encoding="utf-8")
        suffix = contents.rsplit(")", 1)[1].strip().split()
        if len(suffix) < 20:
            raise RuntimeError(f"incomplete process metadata for PID {pid}")
        return LinuxProcessInfo(
            pid=pid,
            state=suffix[0],
            ppid=int(suffix[1]),
            start_time=int(suffix[19]),
        )
    except (IndexError, ValueError) as error:
        raise RuntimeError(f"invalid process metadata for PID {pid}") from error


def linux_process_table(
    *, max_entries: int = MAX_LINUX_PROCESS_TABLE_ENTRIES
) -> dict[int, LinuxProcessInfo]:
    """Read the same-user process table without trusting mutable environments."""

    proc = Path("/proc")
    try:
        (proc / "self/stat").read_text(encoding="utf-8")
    except OSError as error:
        raise RuntimeError("Linux /proc process supervision is unavailable") from error
    processes: dict[int, LinuxProcessInfo] = {}
    try:
        numeric_entries = 0
        for entry in proc.iterdir():
            if not entry.name.isdigit():
                continue
            numeric_entries += 1
            if numeric_entries > max_entries:
                raise LinuxProcessTableLimit(int(entry.name))
            try:
                if entry.stat().st_uid != os.getuid():
                    continue
                pid = int(entry.name)
                processes[pid] = linux_process_info(pid)
            except (FileNotFoundError, ProcessLookupError):
                continue
            except PermissionError as error:
                raise RuntimeError(
                    f"Linux /proc process metadata is unreadable for same-user PID {entry.name}"
                ) from error
            except (IndexError, ValueError) as error:
                raise RuntimeError(f"invalid process metadata for PID {entry.name}") from error
    except OSError as error:
        raise RuntimeError("Linux /proc process supervision is unavailable") from error
    if os.getpid() not in processes:
        raise RuntimeError("Linux /proc did not report the benchmark supervisor")
    return processes


def linux_descendant_processes(
    table: Mapping[int, LinuxProcessInfo],
    known_identities: Sequence[tuple[int, int]],
    root_identity: tuple[int, int] | None,
    owner_pid: int,
    baseline_children: set[tuple[int, int]],
    *,
    max_processes: int = MAX_LINUX_CONTAINED_PROCESSES,
) -> list[LinuxProcessInfo]:
    """Find contained identities with one bounded adjacency build and BFS."""

    children: dict[int, list[LinuxProcessInfo]] = {}
    adopted: list[LinuxProcessInfo] = []
    for info in table.values():
        children.setdefault(info.ppid, []).append(info)
        identity = (info.pid, info.start_time)
        if (
            info.ppid == owner_pid
            and identity not in baseline_children
            and info.pid != owner_pid
        ):
            adopted.append(info)

    discovered: set[tuple[int, int]] = set()
    frontier: list[LinuxProcessInfo] = []

    def admit(info: LinuxProcessInfo) -> None:
        identity = (info.pid, info.start_time)
        if identity in discovered:
            return
        if len(discovered) >= max_processes:
            raise LinuxContainmentProcessLimit(frontier, info)
        discovered.add(identity)
        frontier.append(info)

    for pid, start_time in known_identities:
        info = table.get(pid)
        if info is not None and info.start_time == start_time:
            admit(info)
    if root_identity is not None:
        root_pid, root_start_time = root_identity
        root = table.get(root_pid)
        if root is not None and root.start_time == root_start_time:
            admit(root)
    for info in adopted:
        admit(info)

    cursor = 0
    while cursor < len(frontier):
        parent = frontier[cursor]
        cursor += 1
        for child in children.get(parent.pid, ()):
            admit(child)
    return frontier


def enable_linux_subreaper() -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    result = libc.prctl(36, 1, 0, 0, 0)  # PR_SET_CHILD_SUBREAPER
    if result != 0:
        error_number = ctypes.get_errno()
        raise RuntimeError(
            f"could not enable Linux child-subreaper containment: {os.strerror(error_number)}"
        )
    enabled = ctypes.c_int()
    result = libc.prctl(37, ctypes.byref(enabled), 0, 0, 0)  # PR_GET_CHILD_SUBREAPER
    if result != 0 or enabled.value != 1:
        raise RuntimeError("Linux child-subreaper containment could not be verified")


class LinuxProcessSupervisor:
    """Track a command's PID identities across setsid, reparenting, and double forks."""

    def __init__(
        self,
        baseline_children: set[tuple[int, int]],
    ) -> None:
        self.root_pid: int | None = None
        self.root_identity: tuple[int, int] | None = None
        self.owner_pid = os.getpid()
        self.baseline_children = baseline_children
        self._known: dict[tuple[int, int], int] = {}
        self._discovery_incomplete: set[tuple[int, int]] = set()
        self._table_scan_incomplete: set[int] = set()
        self._lock = threading.Lock()

    @staticmethod
    def capture_baseline() -> set[tuple[int, int]]:
        table = linux_process_table()
        return {
            (info.pid, info.start_time)
            for info in table.values()
            if info.ppid == os.getpid()
        }

    def _open_pidfd(self, info: LinuxProcessInfo) -> None:
        identity = (info.pid, info.start_time)
        if identity in self._known:
            return
        try:
            descriptor = os.pidfd_open(info.pid, 0)
        except ProcessLookupError:
            return
        try:
            current = linux_process_info(info.pid)
        except (FileNotFoundError, ProcessLookupError):
            os.close(descriptor)
            return
        if current.start_time != info.start_time:
            os.close(descriptor)
            return
        self._known[identity] = descriptor

    def attach_root(self, root_pid: int, descriptor: int | None = None) -> None:
        """Attach to the direct child in O(1), without a process-table scan."""

        owns_descriptor = descriptor is None
        if descriptor is None:
            descriptor = os.pidfd_open(root_pid, 0)
        try:
            root = linux_process_info(root_pid)
        except BaseException:
            if owns_descriptor:
                os.close(descriptor)
            raise
        if root.ppid != self.owner_pid:
            if owns_descriptor:
                os.close(descriptor)
            raise RuntimeError("launched process is not a direct supervisor child")
        self.root_pid = root_pid
        self.root_identity = (root.pid, root.start_time)
        with self._lock:
            self._known[self.root_identity] = descriptor

    def refresh(self) -> dict[int, LinuxProcessInfo]:
        try:
            table = linux_process_table()
        except LinuxProcessTableLimit as error:
            with self._lock:
                incomplete = getattr(self, "_table_scan_incomplete", set())
                incomplete.add(error.overflow_pid)
                self._table_scan_incomplete = incomplete
            raise
        with self._lock:
            for identity, descriptor in list(self._known.items()):
                pid, start_time = identity
                current = table.get(pid)
                if current is not None and current.start_time == start_time:
                    continue
                try:
                    os.close(descriptor)
                except BaseException:
                    pass
                del self._known[identity]
            limit_error: LinuxContainmentProcessLimit | None = None
            try:
                descendants = linux_descendant_processes(
                    table,
                    tuple(self._known),
                    self.root_identity,
                    self.owner_pid,
                    self.baseline_children,
                )
            except LinuxContainmentProcessLimit as error:
                descendants = error.discovered
                incomplete = getattr(self, "_discovery_incomplete", set())
                incomplete.add((error.overflow.pid, error.overflow.start_time))
                self._discovery_incomplete = incomplete
                limit_error = error
            for info in descendants:
                if (info.pid, info.start_time) not in self._known:
                    self._open_pidfd(info)
            if limit_error is not None:
                raise limit_error
            self._discovery_incomplete = set()
            self._table_scan_incomplete = set()
            return table

    def live_pids(self) -> set[int]:
        table = self.refresh()
        with self._lock:
            return {
                pid
                for pid, start_time in self._known
                if (info := table.get(pid)) is not None
                and info.start_time == start_time
                and info.state != "Z"
            }

    def adopted_pids(self, *, include_zombies: bool) -> set[int]:
        """Return every still-present known descendant, including adopted zombies."""

        table = self.refresh()
        with self._lock:
            return {
                pid
                for pid, start_time in self._known
                if pid != self.root_pid
                and (info := table.get(pid)) is not None
                and info.start_time == start_time
                and (include_zombies or info.state != "Z")
            }

    def signal_known(self, signal_number: int) -> None:
        with self._lock:
            descriptors = list(self._known.values())
        for descriptor in descriptors:
            try:
                signal.pidfd_send_signal(descriptor, signal_number)
            except ProcessLookupError:
                pass

    def reap_adopted(self) -> None:
        with self._lock:
            descriptors = [
                descriptor
                for (pid, _), descriptor in self._known.items()
                if pid != self.root_pid
            ]
        for descriptor in descriptors:
            try:
                os.waitid(os.P_PIDFD, descriptor, os.WEXITED | os.WNOHANG)
            except (ChildProcessError, ProcessLookupError):
                pass

    def known_present_pids(self) -> set[int]:
        """Query immutable pidfds without expanding ancestry or trusting numeric PIDs."""

        with self._lock:
            identities = list(self._known.items())
            incomplete_pids = {
                pid
                for pid, _start_time in getattr(self, "_discovery_incomplete", set())
            }
            incomplete_pids.update(getattr(self, "_table_scan_incomplete", set()))
        present = incomplete_pids
        for (pid, _), descriptor in identities:
            if pid == self.root_pid:
                continue
            try:
                signal.pidfd_send_signal(descriptor, 0)
            except ProcessLookupError:
                continue
            present.add(pid)
        return present

    def settle_and_reap_adopted(self, settle_seconds: float = 0.25) -> set[int]:
        """Bounded post-exit wait that leaves no known adopted child or zombie."""

        deadline = time.monotonic() + settle_seconds
        clean_scans = 0
        while time.monotonic() < deadline:
            self.reap_adopted()
            remaining = self.adopted_pids(include_zombies=True)
            if not remaining:
                clean_scans += 1
                if clean_scans >= 2:
                    return set()
            else:
                clean_scans = 0
            time.sleep(0.01)
        self.reap_adopted()
        return self.adopted_pids(include_zombies=True)

    def stop(self) -> None:
        with defer_harness_signal_while_cleaning():
            with self._lock:
                descriptors = list(self._known.values())
                self._known.clear()
            for descriptor in descriptors:
                close_descriptor_nonthrowing(descriptor)


class LinuxExitObserver:
    """Timestamp pidfd readability without reaping or scanning `/proc`."""

    def __init__(self) -> None:
        self._armed = threading.Event()
        self._registered = threading.Event()
        self._finished = threading.Event()
        self._descriptor: int | None = None
        self._cancelled = False
        self._already_readable = False
        self._error: BaseException | None = None
        self.end_ns: int | None = None
        self._thread = threading.Thread(target=self._observe, daemon=True)
        self._thread.start()

    def arm(self, descriptor: int) -> None:
        self._descriptor = os.dup(descriptor)
        self._armed.set()
        if not self._registered.wait(1.0):
            raise RuntimeError("Linux exit observer registration timed out")
        if self._error is not None:
            raise RuntimeError("Linux exit observer registration failed") from self._error
        if self._already_readable:
            raise RuntimeError(
                "gated process exited before measurement observation was ready"
            )

    def _register_pidfd(self, poller: select.poll, descriptor: int) -> None:
        poller.register(descriptor, select.POLLIN)

    def _observe(self) -> None:
        self._armed.wait()
        if self._cancelled:
            self._registered.set()
            self._finished.set()
            return
        descriptor = self._descriptor
        if descriptor is None:
            self._error = RuntimeError("Linux exit observer was not armed")
            self._finished.set()
            return
        try:
            poller = select.poll()
            self._register_pidfd(poller, descriptor)
            if poller.poll(0):
                self._already_readable = True
                return
            self._registered.set()
            poller.poll()
            self.end_ns = time.perf_counter_ns()
        except BaseException as error:
            self._error = error
        finally:
            self._registered.set()
            os.close(descriptor)
            self._descriptor = None
            self._finished.set()

    def wait(self, deadline_ns: int) -> int | None:
        remaining_ns = deadline_ns - time.perf_counter_ns()
        if remaining_ns > 0:
            self._finished.wait(remaining_ns / 1_000_000_000)
        if not self._finished.is_set():
            return None
        if self._error is not None:
            raise RuntimeError("Linux exit observation failed") from self._error
        return self.end_ns

    def stop(self) -> None:
        if not self._armed.is_set():
            self._cancelled = True
            self._armed.set()
        self._thread.join(timeout=0.5)


class GatedProcess:
    """Minimal Popen-compatible handle for a forked child blocked before exec."""

    stdout = None
    stderr = None

    def __init__(self, pid: int, gate_descriptor: int, command: Sequence[str]) -> None:
        self.pid = pid
        self.gate_descriptor: int | None = gate_descriptor
        self.command = list(command)
        self.returncode: int | None = None

    def release(self) -> None:
        descriptor = self.gate_descriptor
        if descriptor is None:
            raise RuntimeError("measurement gate was already released")
        self.gate_descriptor = None
        try:
            os.write(descriptor, b"\x01")
        finally:
            close_descriptor_nonthrowing(descriptor)

    def close_gate(self) -> None:
        descriptor = self.gate_descriptor
        self.gate_descriptor = None
        close_descriptor_nonthrowing(descriptor)

    def _record_status(self, status: int) -> int:
        self.returncode = os.waitstatus_to_exitcode(status)
        self.close_gate()
        return self.returncode

    def poll(self) -> int | None:
        if self.returncode is not None:
            return self.returncode
        waited_pid, status = os.waitpid(self.pid, os.WNOHANG)
        if waited_pid == 0:
            return None
        return self._record_status(status)

    def wait(self, timeout: float | None = None) -> int:
        if self.returncode is not None:
            return self.returncode
        if timeout is None:
            _, status = os.waitpid(self.pid, 0)
            return self._record_status(status)
        deadline = time.monotonic() + timeout
        while True:
            result = self.poll()
            if result is not None:
                return result
            if time.monotonic() >= deadline:
                raise subprocess.TimeoutExpired(self.command, timeout)
            time.sleep(0.005)

    def kill(self) -> None:
        self.close_gate()
        try:
            os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def close_descriptor_nonthrowing(descriptor: int | None) -> None:
    if descriptor is None:
        return
    try:
        os.close(descriptor)
    except BaseException:
        pass


def kill_and_reap_child_nonthrowing(pid: int, timeout_seconds: float = 1.0) -> None:
    """Best-effort bounded kill and reap used while preserving a launch exception."""

    try:
        os.kill(pid, signal.SIGKILL)
    except BaseException:
        pass
    try:
        deadline = time.monotonic() + timeout_seconds
    except BaseException:
        deadline = 0.0
    for _ in range(max(1, math.ceil(timeout_seconds / 0.005))):
        try:
            waited_pid, _ = os.waitpid(pid, os.WNOHANG)
            if waited_pid == pid:
                return
        except ChildProcessError:
            return
        except BaseException:
            pass
        try:
            if time.monotonic() >= deadline:
                break
        except BaseException:
            pass
        try:
            time.sleep(0.005)
        except BaseException:
            pass
    try:
        os.waitpid(pid, os.WNOHANG)
    except BaseException:
        pass


def launch_gated_process(
    command: Sequence[str],
    cwd: Path,
    environment: Mapping[str, str],
    executable_descriptor: int | None,
) -> GatedProcess:
    """Fork a new process group whose target exec waits behind a private gate."""

    if executable_descriptor is not None and os.execve not in os.supports_fd:
        raise RuntimeError("Linux measurement requires descriptor-based execve")
    pipe_flags = getattr(os, "O_CLOEXEC", 0)
    gate_read: int | None = None
    gate_write: int | None = None
    ready_read: int | None = None
    ready_write: int | None = None
    pid: int | None = None
    try:
        gate_read, gate_write = os.pipe2(pipe_flags)
        ready_read, ready_write = os.pipe2(pipe_flags)
        pid = os.fork()
        if pid == 0:
            try:
                os.close(gate_write)
                os.close(ready_read)
                os.setsid()
                os.chdir(cwd)
                devnull = os.open(os.devnull, os.O_RDWR)
                try:
                    for descriptor in (0, 1, 2):
                        os.dup2(devnull, descriptor)
                finally:
                    if devnull > 2:
                        os.close(devnull)
                os.write(ready_write, b"\x01")
                os.close(ready_write)
                released = os.read(gate_read, 1)
                os.close(gate_read)
                if released != b"\x01":
                    os._exit(126)
                arguments = list(command)
                child_environment = dict(environment)
                if executable_descriptor is None:
                    os.execve(arguments[0], arguments, child_environment)
                os.execve(executable_descriptor, arguments, child_environment)
            except BaseException:
                os._exit(127)
        os.close(gate_read)
        gate_read = None
        os.close(ready_write)
        ready_write = None
        poller = select.poll()
        poller.register(ready_read, select.POLLIN | select.POLLHUP)
        ready = bool(poller.poll(1000)) and os.read(ready_read, 1) == b"\x01"
        os.close(ready_read)
        ready_read = None
        if not ready:
            raise RuntimeError("gated measurement child did not reach the exec barrier")
        process = GatedProcess(pid, gate_write, command)
        gate_write = None
        return process
    except BaseException:
        close_descriptor_nonthrowing(gate_read)
        close_descriptor_nonthrowing(gate_write)
        close_descriptor_nonthrowing(ready_read)
        close_descriptor_nonthrowing(ready_write)
        if pid is not None and pid > 0:
            kill_and_reap_child_nonthrowing(pid)
        raise


def kill_process_group(pid: int, signal_number: int) -> None:
    if os.name != "posix":
        return
    try:
        os.killpg(pid, signal_number)
    except OSError:
        pass


def linux_containment_preflight() -> None:
    """Prove this kernel can discover and kill a hostile detached grandchild."""

    global _LINUX_PREFLIGHT_COMPLETE
    if not sys.platform.startswith("linux") or _LINUX_PREFLIGHT_COMPLETE:
        return
    with _LINUX_PREFLIGHT_LOCK:
        if _LINUX_PREFLIGHT_COMPLETE:
            return
        if (
            not hasattr(os, "pidfd_open")
            or not hasattr(os, "P_PIDFD")
            or not hasattr(signal, "pidfd_send_signal")
            or not hasattr(select, "poll")
        ):
            raise RuntimeError("Linux containment requires pidfd support")
        enable_linux_subreaper()
        baseline = LinuxProcessSupervisor.capture_baseline()
        with tempfile.TemporaryDirectory(prefix="machine-god-containment-") as directory:
            marker = Path(directory) / "hostile.pid"
            marker_staging = Path(directory) / "hostile.pid.partial"
            script = (
                "import os,pathlib,time; "
                "first=os.fork(); "
                "(os._exit(0) if first else None); "
                "os.setsid(); second=os.fork(); "
                "(os._exit(0) if second else None); "
                "os.environ.clear(); "
                f"marker=pathlib.Path({str(marker)!r}); "
                f"staging=pathlib.Path({str(marker_staging)!r}); "
                "staging.write_text(str(os.getpid()), encoding='ascii'); "
                "os.replace(staging, marker); "
                "time.sleep(30)"
            )
            supervisor = LinuxProcessSupervisor(baseline)
            process: subprocess.Popen[bytes] | None = None
            try:
                # A termination arriving after Popen has returned but before the
                # root pidfd belongs to the supervisor must not unwind through
                # an untracked child.  Deferral uses a Python handler rather
                # than a blocked signal mask, so the child inherits no mask.
                with defer_harness_signal_while_spawning():
                    process = subprocess.Popen(
                        [sys.executable, "-c", script],
                        stdin=subprocess.DEVNULL,
                        stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL,
                        start_new_session=True,
                    )
                    supervisor.attach_root(process.pid)
                deadline = time.monotonic() + 2.0
                while not marker.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                if not marker.exists():
                    raise RuntimeError("Linux containment preflight child did not start")
                hostile_pid = int(marker.read_text(encoding="utf-8"))
                if hostile_pid not in supervisor.live_pids():
                    raise RuntimeError("Linux containment did not discover a hostile grandchild")
                remaining = terminate_contained_process(process, supervisor)
                surface_deferred_harness_signal()
                if remaining:
                    raise RuntimeError(
                        f"Linux containment could not kill hostile PIDs {sorted(remaining)}"
                    )
            finally:
                if process is not None:
                    terminate_contained_process(process, supervisor)
                    surface_deferred_harness_signal()
                else:
                    supervisor.stop()
        _LINUX_PREFLIGHT_COMPLETE = True


class BoundedProcessCapture:
    """Drain both process pipes concurrently without retaining unbounded bytes."""

    def __init__(self, process: subprocess.Popen[bytes], limit_bytes: int) -> None:
        if process.stdout is None or process.stderr is None:
            raise RuntimeError("captured process pipes are unavailable")
        self._streams: dict[str, BinaryIO] = {
            "stdout": process.stdout,
            "stderr": process.stderr,
        }
        self._limit_bytes = limit_bytes
        self._buffers = {name: bytearray() for name in self._streams}
        self._finished = {name: threading.Event() for name in self._streams}
        self._changed = threading.Event()
        self._stopping = threading.Event()
        self._lock = threading.Lock()
        self._overflow: tuple[str, int] | None = None
        self._errors: dict[str, BaseException] = {}
        self._threads = [
            threading.Thread(
                target=self._drain,
                args=(name, stream),
                name=f"machine-god-capture-{name}",
                daemon=True,
            )
            for name, stream in self._streams.items()
        ]
        for thread in self._threads:
            thread.start()

    def _drain(self, name: str, stream: BinaryIO) -> None:
        buffer = self._buffers[name]
        try:
            descriptor = stream.fileno()
            while True:
                remaining = self._limit_bytes - len(buffer)
                requested = min(PROCESS_OUTPUT_READ_BYTES, remaining + 1)
                chunk = os.read(descriptor, requested)
                if not chunk:
                    break
                observed = len(buffer) + len(chunk)
                if observed > self._limit_bytes:
                    if remaining:
                        buffer.extend(chunk[:remaining])
                    with self._lock:
                        if self._overflow is None:
                            self._overflow = (name, observed)
                    break
                buffer.extend(chunk)
        except BaseException as error:
            if not self._stopping.is_set():
                with self._lock:
                    self._errors[name] = error
        finally:
            self._finished[name].set()
            self._changed.set()

    def wait(
        self,
        process: subprocess.Popen[bytes],
        command: Sequence[str],
        deadline_ns: int,
    ) -> tuple[bytes, bytes, int]:
        while True:
            with self._lock:
                overflow = self._overflow
                errors = dict(self._errors)
            if overflow is not None:
                name, observed = overflow
                raise ProcessOutputLimit(
                    name,
                    self._limit_bytes,
                    observed,
                    command,
                )
            if errors:
                name, error = next(iter(errors.items()))
                raise RuntimeError(f"failed to capture process {name}") from error
            if all(finished.is_set() for finished in self._finished.values()):
                returncode = process.poll()
                if returncode is not None:
                    return (
                        bytes(self._buffers["stdout"]),
                        bytes(self._buffers["stderr"]),
                        time.perf_counter_ns(),
                    )
            remaining_ns = deadline_ns - time.perf_counter_ns()
            if remaining_ns <= 0:
                raise subprocess.TimeoutExpired(command, 0)
            self._changed.wait(min(remaining_ns / 1_000_000_000, 0.01))
            self._changed.clear()

    def stop(self) -> None:
        self._stopping.set()
        for stream in self._streams.values():
            try:
                stream.close()
            except OSError:
                pass
        deadline = time.monotonic() + 1.0
        for thread in self._threads:
            thread.join(max(0.0, deadline - time.monotonic()))
        if any(thread.is_alive() for thread in self._threads):
            raise RuntimeError("process output capture threads did not stop")


def close_process_pipes(process: subprocess.Popen[bytes] | GatedProcess) -> None:
    for name in ("stdout", "stderr"):
        try:
            stream = getattr(process, name)
            if stream is not None and not stream.closed:
                stream.close()
        except BaseException:
            pass


def terminate_contained_process(
    process: subprocess.Popen[bytes] | GatedProcess,
    supervisor: LinuxProcessSupervisor | None,
    cleanup_seconds: float = 2.0,
) -> set[int]:
    """Non-throwing bounded cleanup for every path after a process launches."""

    remaining: set[int] = set()
    process_pid: int | None = None
    with defer_harness_signal_while_cleaning():
        try:
            process_pid = process.pid
        except BaseException:
            pass
        try:
            cleanup_iterations = max(1, math.ceil(cleanup_seconds / 0.01))
        except BaseException:
            cleanup_iterations = 1
        try:
            deadline = time.monotonic() + cleanup_seconds
        except BaseException:
            deadline = 0.0
        if process_pid is not None:
            try:
                kill_process_group(process_pid, signal.SIGTERM)
            except BaseException:
                pass
        try:
            close_process_pipes(process)
        except BaseException:
            pass
        for _ in range(cleanup_iterations):
            if process_pid is not None:
                try:
                    kill_process_group(process_pid, signal.SIGKILL)
                except BaseException:
                    pass
            root_running = True
            try:
                root_running = process.poll() is None
            except BaseException:
                pass
            if root_running:
                try:
                    process.kill()
                except BaseException:
                    pass
            if supervisor is not None:
                try:
                    supervisor.refresh()
                except BaseException:
                    pass
                try:
                    supervisor.signal_known(signal.SIGKILL)
                except BaseException:
                    pass
                try:
                    supervisor.reap_adopted()
                except BaseException:
                    pass
                try:
                    remaining = set(supervisor.known_present_pids())
                except BaseException:
                    pass
            try:
                root_finished = process.poll() is not None
            except BaseException:
                root_finished = False
            if root_finished and not remaining:
                break
            try:
                if time.monotonic() >= deadline:
                    break
            except BaseException:
                pass
            try:
                time.sleep(0.01)
            except BaseException:
                pass
        try:
            remaining_seconds = max(0.01, deadline - time.monotonic())
        except BaseException:
            remaining_seconds = 0.01
        try:
            process.wait(timeout=remaining_seconds)
        except BaseException:
            pass
        if supervisor is not None:
            try:
                supervisor.reap_adopted()
            except BaseException:
                pass
            try:
                remaining = set(supervisor.known_present_pids())
            except BaseException:
                pass
            try:
                supervisor.stop()
            except BaseException:
                pass
        root_present = True
        try:
            root_present = process.poll() is None
        except BaseException:
            pass
        if root_present and process_pid is not None:
            remaining.add(process_pid)
    return remaining


def finalize_successful_process(
    process: subprocess.Popen[bytes] | GatedProcess,
    supervisor: LinuxProcessSupervisor | None,
) -> None:
    """Check containment after timing has ended and reject leaked descendants."""

    if supervisor is None:
        return
    try:
        leaked = supervisor.settle_and_reap_adopted()
    except BaseException:
        terminate_contained_process(process, supervisor)
        surface_deferred_harness_signal()
        raise
    if leaked:
        remaining = terminate_contained_process(process, supervisor)
        surface_deferred_harness_signal()
        raise RuntimeError(
            "command left detached descendants"
            + (f" and containment is incomplete for PIDs {sorted(remaining)}" if remaining else "")
        )
    supervisor.stop()
    surface_deferred_harness_signal()


def run_process(
    command: Sequence[str],
    *,
    cwd: Path,
    environment: Mapping[str, str],
    timeout_seconds: float,
    capture_output: bool = True,
    max_output_bytes: int = DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
    expected_executable: Mapping[str, object] | None = None,
    executable_descriptor: int | None = None,
    executable_path: Path | None = None,
) -> ProcessResult:
    setup_started = time.perf_counter_ns()
    if not is_positive_number(timeout_seconds):
        raise ValueError("process timeout must be a positive finite number")
    if not is_integer(max_output_bytes) or max_output_bytes <= 0:
        raise ValueError("process output limit must be a positive integer")
    process_environment = dict(environment)
    supervisor: LinuxProcessSupervisor | None = None
    exit_observer: LinuxExitObserver | None = None
    linux = sys.platform.startswith("linux")
    gated_measurement = linux and not capture_output
    if linux:
        if CONTAINMENT_ENVIRONMENT_KEY not in process_environment:
            raise RuntimeError("Linux subprocess execution requires a containment token")
        linux_containment_preflight()
        baseline_children = LinuxProcessSupervisor.capture_baseline()
        supervisor = LinuxProcessSupervisor(baseline_children)
    elif executable_descriptor is not None:
        raise RuntimeError("descriptor-based measurement execution requires Linux")
    if expected_executable is not None:
        verify_executable_identity(expected_executable)
    process: subprocess.Popen[bytes] | GatedProcess | None = None
    output_capture: BoundedProcessCapture | None = None
    root_descriptor: int | None = None
    supervision_ns = 0
    setup_ns = 0
    start = 0
    deadline_ns = 0
    if not gated_measurement:
        setup_ns = time.perf_counter_ns() - setup_started
        start = time.perf_counter_ns()
        deadline_ns = start + int(timeout_seconds * 1_000_000_000)
    try:
        with defer_harness_signal_while_spawning():
            if gated_measurement:
                process = launch_gated_process(
                    command,
                    cwd,
                    process_environment,
                    executable_descriptor,
                )
                exit_observer = LinuxExitObserver()
            else:
                process = subprocess.Popen(
                    list(command),
                    cwd=cwd,
                    env=process_environment,
                    executable=str(executable_path) if executable_path is not None else None,
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE if capture_output else subprocess.DEVNULL,
                    stderr=subprocess.PIPE if capture_output else subprocess.DEVNULL,
                    start_new_session=True,
                )
                if capture_output:
                    output_capture = BoundedProcessCapture(process, max_output_bytes)
            if supervisor is not None:
                supervision_started = time.perf_counter_ns()
                root_descriptor = os.pidfd_open(process.pid, 0)
                if exit_observer is not None:
                    exit_observer.arm(root_descriptor)
                supervisor.attach_root(process.pid, root_descriptor)
                root_descriptor = None
                supervision_ns = time.perf_counter_ns() - supervision_started
        if isinstance(process, GatedProcess):
            setup_ns = time.perf_counter_ns() - setup_started
            start = time.perf_counter_ns()
            deadline_ns = start + int(timeout_seconds * 1_000_000_000)
            process.release()
        try:
            if exit_observer is not None:
                end = exit_observer.wait(deadline_ns)
                if end is None or end > deadline_ns:
                    raise subprocess.TimeoutExpired(command, timeout_seconds)
                process.wait()
                stdout, stderr = b"", b""
            elif output_capture is not None:
                stdout, stderr, end = output_capture.wait(
                    process,
                    command,
                    deadline_ns,
                )
                output_capture.stop()
                output_capture = None
            else:
                remaining_ns = deadline_ns - time.perf_counter_ns()
                if remaining_ns <= 0:
                    raise subprocess.TimeoutExpired(command, timeout_seconds)
                stdout, stderr = process.communicate(
                    timeout=remaining_ns / 1_000_000_000
                )
                end = time.perf_counter_ns()
        except subprocess.TimeoutExpired as error:
            remaining = terminate_contained_process(process, supervisor)
            surface_deferred_harness_signal()
            if exit_observer is not None:
                exit_observer.stop()
            if expected_executable is not None:
                verify_executable_identity(expected_executable)
            detail = (
                f"; containment incomplete for PIDs {sorted(remaining)}"
                if remaining
                else ""
            )
            raise ProcessTimeout(
                f"command timed out after {timeout_seconds}s: {' '.join(command)}{detail}"
            ) from error
        except ProcessOutputLimit as error:
            remaining = terminate_contained_process(process, supervisor)
            surface_deferred_harness_signal()
            if expected_executable is not None:
                verify_executable_identity(expected_executable)
            if remaining:
                raise RuntimeError(
                    f"{error}; containment incomplete for PIDs {sorted(remaining)}"
                ) from error
            raise
        cleanup_started = time.perf_counter_ns()
        finalize_successful_process(process, supervisor)
        if expected_executable is not None:
            verify_executable_identity(expected_executable)
        cleanup_ns = time.perf_counter_ns() - cleanup_started
        if exit_observer is not None:
            exit_observer.stop()
        return ProcessResult(
            returncode=process.returncode,
            stdout=stdout or b"",
            stderr=stderr or b"",
            elapsed_ns=end - start,
            setup_ns=setup_ns,
            supervision_ns=supervision_ns,
            cleanup_ns=cleanup_ns,
        )
    except BaseException:
        if process is not None:
            terminate_contained_process(process, supervisor)
        elif supervisor is not None:
            supervisor.stop()
        if exit_observer is not None:
            exit_observer.stop()
        surface_deferred_harness_signal()
        raise
    finally:
        if output_capture is not None:
            try:
                output_capture.stop()
            except BaseException:
                pass
        descriptor = root_descriptor
        root_descriptor = None
        close_descriptor_nonthrowing(descriptor)


def invocation_path(command: str, path_value: str) -> str:
    executable = shutil.which(command, path=path_value)
    if executable is None:
        raise RuntimeError(f"required executable was not found: {command}")
    path = Path(executable).absolute()
    path.resolve(strict=True)
    return str(path)


def tool_record(
    command: list[str],
    *,
    environment: Mapping[str, str],
    timeout_seconds: float,
    required_version: str | None = None,
) -> dict[str, object]:
    identity = executable_identity(Path(command[0]))
    completed = run_process(
        command,
        cwd=Path.cwd(),
        environment=environment,
        timeout_seconds=timeout_seconds,
        expected_executable=identity,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode(errors="replace").strip() or completed.stdout.decode(
            errors="replace"
        ).strip()
        raise RuntimeError(f"tool version command failed ({' '.join(command)}): {detail}")
    version = (
        completed.stdout.decode(errors="replace").strip()
        or completed.stderr.decode(errors="replace").strip()
    )
    record: dict[str, object] = {
        **identity,
        "command": command,
        "version": version,
    }
    if required_version is not None:
        record["required_version"] = required_version
    return record


def verify_tool_versions(
    git: str,
    zig: str,
    rustc: str,
    cargo: str,
    *,
    environment: Mapping[str, str],
    timeout_seconds: float,
) -> dict[str, object]:
    tools = {
        "git": tool_record(
            [git, "--version"],
            environment=environment,
            timeout_seconds=timeout_seconds,
        ),
        "zig": tool_record(
            [zig, "version"],
            environment=environment,
            timeout_seconds=timeout_seconds,
            required_version=EXPECTED_ZIG_VERSION,
        ),
        "rustc": tool_record(
            [rustc, f"+{EXPECTED_RUST_VERSION}", "--version"],
            environment=environment,
            timeout_seconds=timeout_seconds,
            required_version=EXPECTED_RUST_VERSION,
        ),
        "cargo": tool_record(
            [cargo, f"+{EXPECTED_RUST_VERSION}", "--version"],
            environment=environment,
            timeout_seconds=timeout_seconds,
            required_version=EXPECTED_RUST_VERSION,
        ),
    }
    if tools["zig"]["version"] != EXPECTED_ZIG_VERSION:
        raise RuntimeError(
            f"Zig {EXPECTED_ZIG_VERSION} is required; found {tools['zig']['version']}"
        )
    for name in ("rustc", "cargo"):
        version = str(tools[name]["version"])
        if not version.startswith(f"{name} {EXPECTED_RUST_VERSION} "):
            raise RuntimeError(f"{name} {EXPECTED_RUST_VERSION} is required; found {version}")
    return tools


def command_result_record(
    command: list[str],
    cwd: Path,
    environment: Mapping[str, str],
    timeout_seconds: float,
    completed: ProcessResult,
) -> dict[str, object]:
    return {
        "command": command,
        "cwd": str(cwd),
        "environment": dict(environment),
        "timeout_seconds": timeout_seconds,
        "elapsed_ns": completed.elapsed_ns,
        "setup_ns": completed.setup_ns,
        "supervision_ns": completed.supervision_ns,
        "cleanup_ns": completed.cleanup_ns,
        "returncode": completed.returncode,
        "stdout_sha256": sha256_bytes(completed.stdout),
        "stderr_sha256": sha256_bytes(completed.stderr),
    }


def run_record(
    command: list[str],
    cwd: Path,
    *,
    environment: Mapping[str, str],
    timeout_seconds: float,
    expected_executable: Mapping[str, object] | None = None,
) -> dict[str, object]:
    completed = run_process(
        command,
        cwd=cwd,
        environment=environment,
        timeout_seconds=timeout_seconds,
        expected_executable=expected_executable,
    )
    if completed.stdout:
        sys.stdout.buffer.write(completed.stdout)
        sys.stdout.buffer.flush()
    if completed.stderr:
        sys.stderr.buffer.write(completed.stderr)
        sys.stderr.buffer.flush()
    record = command_result_record(command, cwd, environment, timeout_seconds, completed)
    if completed.returncode != 0:
        raise RuntimeError(f"command exited {completed.returncode}: {' '.join(command)}")
    return record


def git_output(
    git: str,
    cwd: Path,
    environment: Mapping[str, str],
    timeout_seconds: float,
    *arguments: str,
    expected_executable: Mapping[str, object] | None = None,
    max_output_bytes: int = DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
) -> str:
    completed = run_process(
        [*git_prefix(git), *arguments],
        cwd=cwd,
        environment=environment,
        timeout_seconds=timeout_seconds,
        max_output_bytes=max_output_bytes,
        expected_executable=expected_executable,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode(errors="replace").strip()
        raise RuntimeError(f"Git command failed: {detail}")
    return completed.stdout.decode(errors="strict").strip()


def check_machine_cleanliness(
    root: Path,
    git: str,
    environment: Mapping[str, str],
    timeout_seconds: float,
    expected_executable: Mapping[str, object] | None = None,
) -> None:
    completed = run_process(
        [
            *git_prefix(git),
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
        ],
        cwd=root,
        environment=environment,
        timeout_seconds=timeout_seconds,
        expected_executable=expected_executable,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode(errors="replace").strip()
        raise RuntimeError(f"Git status command failed: {detail}")
    rejected: list[str] = []
    allowed_prefixes = tuple(prefix.encode() for prefix in ALLOWED_MACHINE_OUTPUTS)
    for entry in parse_porcelain_v1_z(completed.stdout):
        path = entry.path[:-1] if entry.path.endswith(b"/") else entry.path
        allowed = any(
            path == prefix or path.startswith(prefix + b"/")
            for prefix in allowed_prefixes
        )
        if entry.state not in {"??", "!!"} or entry.original_path is not None or not allowed:
            description = f"{entry.state} {os.fsdecode(entry.path)!r}"
            if entry.original_path is not None:
                description += f" from {os.fsdecode(entry.original_path)!r}"
            rejected.append(description)
    if rejected:
        raise RuntimeError(
            "machine-god worktree contains non-output changes or untracked inputs: "
            + "; ".join(rejected)
        )


def parse_porcelain_v1_z(status: bytes) -> list[MachineStatusEntry]:
    """Parse byte-exact `git status --porcelain=v1 -z` output."""

    if not status:
        return []
    if not status.endswith(b"\0"):
        raise RuntimeError("Git status -z output is not NUL terminated")
    records = status.split(b"\0")[:-1]
    entries: list[MachineStatusEntry] = []
    index = 0
    while index < len(records):
        record = records[index]
        index += 1
        if len(record) < 4 or record[2:3] != b" ":
            raise RuntimeError("Git status -z output contains a malformed record")
        try:
            state = record[:2].decode("ascii", errors="strict")
        except UnicodeDecodeError as error:
            raise RuntimeError("Git status -z output contains a malformed state") from error
        path = record[3:]
        if not path:
            raise RuntimeError("Git status -z output contains an empty path")
        original_path: bytes | None = None
        if "R" in state or "C" in state:
            if index >= len(records) or not records[index]:
                raise RuntimeError("Git status -z rename/copy record is incomplete")
            original_path = records[index]
            index += 1
        entries.append(MachineStatusEntry(state, path, original_path))
    return entries


def machine_tree_command(git: str, commit: str) -> list[str]:
    return [*git_prefix(git), "ls-tree", "-r", "-z", "--full-tree", commit]


def git_blob_command(git: str, object_id: str) -> list[str]:
    return [*git_prefix(git), "cat-file", "blob", object_id]


def parse_git_tree_listing(listing: bytes) -> list[dict[str, object]]:
    entries: list[dict[str, object]] = []
    for raw_entry in listing.split(b"\0"):
        if not raw_entry:
            continue
        try:
            metadata, raw_path = raw_entry.split(b"\t", 1)
            mode, object_type, object_id = metadata.decode("ascii").split(" ", 2)
            path = raw_path.decode("utf-8", errors="strict")
        except (UnicodeDecodeError, ValueError) as error:
            raise RuntimeError("Git tree contains an unparseable entry") from error
        parts = path.split("/")
        if not path or path.startswith("/") or any(part in {"", ".", ".."} for part in parts):
            raise RuntimeError(f"Git tree contains an unsafe path: {path!r}")
        if object_type != "blob" or mode not in {"100644", "100755"}:
            raise RuntimeError(
                f"Git tree contains unsupported mode/type {mode} {object_type}: {path}"
            )
        if not HEX_SHA_RE.fullmatch(object_id):
            raise RuntimeError(f"Git tree contains an invalid object ID: {path}")
        entries.append({"path": path, "mode": mode, "object": object_id})
    if not entries:
        raise RuntimeError("Git tree contains no regular files")
    paths = [str(entry["path"]) for entry in entries]
    if paths != sorted(paths) or len(paths) != len(set(paths)):
        raise RuntimeError("Git tree entries are duplicated or not canonical")
    return entries


def canonical_json_bytes(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode(
        "utf-8"
    )


def canonical_git_entries_sha256(entries: Sequence[Mapping[str, object]]) -> str:
    projected = [
        {"mode": entry["mode"], "object": entry["object"], "path": entry["path"]}
        for entry in entries
    ]
    return sha256_bytes(canonical_json_bytes(projected))


def canonical_manifest_sha256(entries: Sequence[Mapping[str, object]]) -> str:
    return sha256_bytes(canonical_json_bytes(list(entries)))


def git_blob_oid(contents: bytes) -> str:
    header = f"blob {len(contents)}\0".encode("ascii")
    return hashlib.sha1(header + contents, usedforsecurity=False).hexdigest()


def bounded_descriptor_bytes(descriptor: int, expected_bytes: int) -> bytes:
    chunks: list[bytes] = []
    offset = 0
    while offset < expected_bytes:
        chunk = os.pread(
            descriptor,
            min(expected_bytes - offset, 1024 * 1024),
            offset,
        )
        if not chunk:
            raise RuntimeError("materialized file became shorter while read")
        chunks.append(chunk)
        offset += len(chunk)
    if os.pread(descriptor, 1, offset):
        raise RuntimeError("materialized file became longer while read")
    return b"".join(chunks)


def validate_source_manifest(value: object, field: str) -> list[dict[str, object]]:
    if not isinstance(value, list) or not value:
        raise ValueError(f"{field} must be a non-empty list")
    entries: list[dict[str, object]] = []
    for index, item in enumerate(value):
        item_field = f"{field}[{index}]"
        if not isinstance(item, dict) or set(item) != {
            "path",
            "mode",
            "object",
            "bytes",
            "sha256",
        }:
            raise ValueError(f"{item_field} has noncanonical fields")
        path = require_text(item.get("path"), f"{item_field}.path")
        parts = path.split("/")
        if path.startswith("/") or any(part in {"", ".", ".."} for part in parts):
            raise ValueError(f"{item_field}.path is unsafe")
        if not isinstance(item.get("mode"), str) or item["mode"] not in {
            "100644",
            "100755",
        }:
            raise ValueError(f"{item_field}.mode is unsupported")
        object_id = require_text(item.get("object"), f"{item_field}.object")
        if not HEX_SHA_RE.fullmatch(object_id):
            raise ValueError(f"{item_field}.object is not a Git blob ID")
        if not is_integer(item.get("bytes")) or item["bytes"] < 0:
            raise ValueError(f"{item_field}.bytes must be a non-negative integer")
        checksum = require_text(item.get("sha256"), f"{item_field}.sha256")
        if len(checksum) != 64 or any(
            character not in "0123456789abcdef" for character in checksum
        ):
            raise ValueError(f"{item_field}.sha256 is not SHA-256")
        entries.append(item)
    paths = [str(entry["path"]) for entry in entries]
    if paths != sorted(paths) or len(paths) != len(set(paths)):
        raise ValueError(f"{field} paths are duplicated or not sorted")
    return entries


def materialized_source_entries(source_dir: Path) -> list[dict[str, object]]:
    entries: list[dict[str, object]] = []
    for path in sorted(
        source_dir.rglob("*"),
        key=lambda candidate: candidate.relative_to(source_dir).as_posix(),
    ):
        relative = path.relative_to(source_dir).as_posix()
        metadata = path.lstat()
        if stat.S_ISDIR(metadata.st_mode):
            continue
        if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
            raise RuntimeError(f"unsupported entry in materialized source: {relative}")
        mode_bits = stat.S_IMODE(metadata.st_mode)
        if mode_bits not in {0o644, 0o755}:
            raise RuntimeError(f"noncanonical mode in materialized source: {relative}")
        open_flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
        open_flags |= getattr(os, "O_CLOEXEC", 0)
        open_flags |= getattr(os, "O_NOFOLLOW", 0)
        open_flags |= getattr(os, "O_NONBLOCK", 0)
        descriptor = os.open(path, open_flags)
        try:
            opened = os.fstat(descriptor)
            if not stat.S_ISREG(opened.st_mode):
                raise RuntimeError(
                    f"unsupported entry in materialized source: {relative}"
                )
            if stat_identity(metadata) != stat_identity(opened):
                raise RuntimeError(
                    f"materialized source path changed before read: {relative}"
                )
            contents = bounded_descriptor_bytes(descriptor, opened.st_size)
            after = os.fstat(descriptor)
            path_after = path.lstat()
            if stat_identity(metadata) != stat_identity(path_after):
                raise RuntimeError(
                    f"materialized source path changed while read: {relative}"
                )
            if stat_identity(opened) != stat_identity(after):
                raise RuntimeError(
                    f"materialized source file changed while read: {relative}"
                )
        finally:
            os.close(descriptor)
        entries.append(
            {
                "path": relative,
                "mode": "100755" if mode_bits == 0o755 else "100644",
                "object": git_blob_oid(contents),
                "bytes": len(contents),
                "sha256": sha256_bytes(contents),
            }
        )
    return entries


def verify_materialized_source(
    source_dir: Path, expected_entries: Sequence[Mapping[str, object]]
) -> str:
    actual_entries = materialized_source_entries(source_dir)
    if actual_entries != list(expected_entries):
        raise RuntimeError("materialized source files, modes, or contents changed")
    return canonical_manifest_sha256(actual_entries)


def source_tree_sha256(source_dir: Path) -> str:
    return canonical_manifest_sha256(materialized_source_entries(source_dir))


def materialize_machine_source(
    root: Path,
    source_dir: Path,
    manifest_path: Path,
    commit: str,
    git: str,
    *,
    environment: Mapping[str, str],
    timeout_seconds: float,
    expected_executable: Mapping[str, object] | None = None,
) -> dict[str, object]:
    if source_dir.exists() or manifest_path.exists():
        raise RuntimeError("fresh machine source materialization paths already exist")
    git_tree = git_output(
        git,
        root,
        environment,
        timeout_seconds,
        "rev-parse",
        f"{commit}^{{tree}}",
        expected_executable=expected_executable,
    )
    if not HEX_SHA_RE.fullmatch(git_tree):
        raise RuntimeError(f"machine-god tree is not a full Git SHA: {git_tree}")
    listing_command = machine_tree_command(git, commit)
    completed = run_process(
        listing_command,
        cwd=root,
        environment=environment,
        timeout_seconds=timeout_seconds,
        expected_executable=expected_executable,
    )
    if completed.returncode != 0:
        raise RuntimeError("Git tree listing failed")
    listing_record = command_result_record(
        listing_command, root, environment, timeout_seconds, completed
    )
    git_entries = parse_git_tree_listing(completed.stdout)
    source_dir.mkdir(parents=True, mode=0o700)
    entries: list[dict[str, object]] = []
    for git_entry in git_entries:
        object_id = str(git_entry["object"])
        blob = run_process(
            git_blob_command(git, object_id),
            cwd=root,
            environment=environment,
            timeout_seconds=timeout_seconds,
            expected_executable=expected_executable,
        )
        if blob.returncode != 0:
            raise RuntimeError(f"could not read Git blob {object_id}")
        if git_blob_oid(blob.stdout) != object_id:
            raise RuntimeError(f"Git returned content that does not match blob {object_id}")
        destination = source_dir / str(git_entry["path"])
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(blob.stdout)
        destination.chmod(0o755 if git_entry["mode"] == "100755" else 0o644)
        entries.append(
            {
                **git_entry,
                "bytes": len(blob.stdout),
                "sha256": sha256_bytes(blob.stdout),
            }
        )
    manifest_contents = canonical_json_bytes(entries)
    manifest_path.write_bytes(manifest_contents)
    source_digest = verify_materialized_source(source_dir, entries)
    return {
        "method": "git-ls-tree-cat-file",
        "source_dir": str(source_dir),
        "manifest_path": str(manifest_path),
        "manifest_sha256": sha256_bytes(manifest_contents),
        "git_entries_sha256": canonical_git_entries_sha256(entries),
        "git_tree": git_tree,
        "source_tree_sha256": source_digest,
        "entries": entries,
        "listing_command": listing_record,
    }


def prepare_upstream(
    root: Path,
    upstream_dir: Path,
    lock: UpstreamLock,
    plan: Mapping[str, list[str]],
    git: str,
    *,
    environment: Mapping[str, str],
    timeout_seconds: float,
    expected_executable: Mapping[str, object] | None = None,
) -> tuple[str, list[dict[str, object]]]:
    if upstream_dir.exists():
        raise RuntimeError(f"fresh upstream checkout path already exists: {upstream_dir}")
    upstream_dir.parent.mkdir(parents=True, exist_ok=True)
    records = [
        run_record(
            plan[name],
            root,
            environment=environment,
            timeout_seconds=timeout_seconds,
            expected_executable=expected_executable,
        )
        for name in ("clone", "fetch", "checkout")
    ]
    hooks_path = git_output(
        git,
        upstream_dir,
        environment,
        timeout_seconds,
        "config",
        "--local",
        "--get",
        "core.hooksPath",
        expected_executable=expected_executable,
    )
    if hooks_path != "/dev/null":
        raise RuntimeError("fresh upstream checkout did not disable Git hooks")
    verified_commit = git_output(
        git,
        upstream_dir,
        environment,
        timeout_seconds,
        "rev-parse",
        "HEAD",
        expected_executable=expected_executable,
    )
    if verified_commit != lock.commit:
        raise RuntimeError(
            f"upstream checkout resolved to {verified_commit}, expected {lock.commit}"
        )
    origin = git_output(
        git,
        upstream_dir,
        environment,
        timeout_seconds,
        "remote",
        "get-url",
        "origin",
        expected_executable=expected_executable,
    )
    if origin != lock.repository:
        raise RuntimeError(f"upstream origin is {origin!r}, expected {lock.repository!r}")
    status = git_output(
        git,
        upstream_dir,
        environment,
        timeout_seconds,
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
        "--ignored",
        expected_executable=expected_executable,
    )
    if status:
        raise RuntimeError(f"fresh upstream checkout contains unexpected files: {status}")
    return verified_commit, records


def binary_record(path: Path) -> dict[str, object]:
    try:
        identity = executable_identity(path)
    except OSError as error:
        raise RuntimeError(f"build did not produce an executable binary: {path}") from error
    return {
        "path": identity["canonical_executable"],
        "bytes": identity["bytes"],
        "sha256": identity["sha256"],
    }


def sha256_descriptor(descriptor: int, expected_bytes: int) -> str:
    digest = hashlib.sha256()
    offset = 0
    while offset < expected_bytes:
        chunk = os.pread(
            descriptor,
            min(expected_bytes - offset, 1024 * 1024),
            offset,
        )
        if not chunk:
            raise RuntimeError("descriptor became shorter while hashed")
        digest.update(chunk)
        offset += len(chunk)
    if os.pread(descriptor, 1, offset):
        raise RuntimeError("descriptor became longer while hashed")
    return digest.hexdigest()


def copy_descriptor(source: int, destination: int, expected_bytes: int) -> None:
    offset = 0
    while offset < expected_bytes:
        chunk = os.pread(
            source,
            min(expected_bytes - offset, 1024 * 1024),
            offset,
        )
        if not chunk:
            raise RuntimeError("source executable became shorter while copied")
        view = memoryview(chunk)
        while view:
            written = os.write(destination, view)
            if written <= 0:
                raise RuntimeError("pinned executable copy made no write progress")
            view = view[written:]
        offset += len(chunk)
    if os.pread(source, 1, offset):
        raise RuntimeError("source executable became longer while copied")


@dataclass
class PinnedExecutable:
    descriptor: int
    method: str
    record: dict[str, object]
    execution_path: Path | None = None
    temporary_directory: tempfile.TemporaryDirectory[str] | None = None

    def verify(self) -> None:
        metadata = os.fstat(self.descriptor)
        for field, actual in (
            ("bytes", metadata.st_size),
            ("mode", stat.S_IMODE(metadata.st_mode)),
            ("device", metadata.st_dev),
            ("inode", metadata.st_ino),
        ):
            if self.record[field] != actual:
                raise RuntimeError(f"pinned executable {field} changed")
        if sha256_descriptor(self.descriptor, int(self.record["bytes"])) != self.record[
            "sha256"
        ]:
            raise RuntimeError("pinned executable content changed")
        metadata_after_hash = os.fstat(self.descriptor)
        if stat_identity(metadata) != stat_identity(metadata_after_hash):
            raise RuntimeError("pinned executable identity changed while hashed")
        if self.method == "linux-sealed-memfd-fexecve":
            expected_seals = int(self.record["seals"])
            if fcntl.fcntl(self.descriptor, fcntl.F_GET_SEALS) != expected_seals:
                raise RuntimeError("pinned executable seals changed")
        elif self.execution_path is not None:
            current = self.execution_path.stat(follow_symlinks=False)
            if (
                current.st_dev != metadata.st_dev
                or current.st_ino != metadata.st_ino
                or not stat.S_ISREG(current.st_mode)
            ):
                raise RuntimeError("private executable copy identity changed")

    def close(self) -> None:
        close_error: BaseException | None = None
        try:
            os.close(self.descriptor)
        except BaseException as error:
            close_error = error
        try:
            if self.temporary_directory is not None:
                self.temporary_directory.cleanup()
        except BaseException as error:
            if close_error is None:
                close_error = error
        if close_error is not None:
            raise close_error


def pin_executable(identity: Mapping[str, object]) -> PinnedExecutable:
    """Copy a verified executable into an immutable per-measurement identity."""

    verify_executable_identity(identity)
    canonical = Path(require_text(identity.get("canonical_executable"), "binary.canonical"))
    expected_bytes = identity.get("bytes")
    if not is_integer(expected_bytes) or expected_bytes <= 0:
        raise RuntimeError("binary identity has an invalid byte count")
    source_flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
    source_flags |= getattr(os, "O_CLOEXEC", 0)
    source_flags |= getattr(os, "O_NOFOLLOW", 0)
    source_flags |= getattr(os, "O_NONBLOCK", 0)
    source: int | None = os.open(canonical, source_flags)
    descriptor: int | None = None
    temporary: tempfile.TemporaryDirectory[str] | None = None
    pinned: PinnedExecutable | None = None
    try:
        source_metadata = os.fstat(source)
        for field, actual in (
            ("bytes", source_metadata.st_size),
            ("mode", stat.S_IMODE(source_metadata.st_mode)),
            ("device", source_metadata.st_dev),
            ("inode", source_metadata.st_ino),
        ):
            if identity.get(field) != actual:
                raise RuntimeError(f"binary identity changed before pinning ({field})")
        canonical_before = canonical.lstat()
        if stat_identity(canonical_before) != stat_identity(source_metadata):
            raise RuntimeError("binary path changed before pinning")
        if sha256_descriptor(source, expected_bytes) != identity.get("sha256"):
            raise RuntimeError("binary content changed before pinning")
        source_after_hash = os.fstat(source)
        if stat_identity(source_metadata) != stat_identity(source_after_hash):
            raise RuntimeError("binary identity changed while verified for pinning")
        if stat_identity(canonical_before) != stat_identity(canonical.lstat()):
            raise RuntimeError("binary path changed while verified for pinning")

        execution_path: Path | None = None
        if sys.platform.startswith("linux"):
            required = (
                "memfd_create",
                "MFD_ALLOW_SEALING",
                "MFD_CLOEXEC",
            )
            if any(not hasattr(os, name) for name in required) or os.execve not in os.supports_fd:
                raise RuntimeError("Linux measurement requires sealed memfd execution")
            descriptor = os.memfd_create(
                "machine-god-benchmark-executable",
                os.MFD_ALLOW_SEALING | os.MFD_CLOEXEC,
            )
            method = "linux-sealed-memfd-fexecve"
        else:
            temporary = tempfile.TemporaryDirectory(prefix="machine-god-pinned-")
            execution_path = Path(temporary.name) / "executable"
            write_descriptor = os.open(
                execution_path,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_CLOEXEC", 0),
                0o700,
            )
            os.close(write_descriptor)
            descriptor = os.open(
                execution_path,
                os.O_RDWR | getattr(os, "O_CLOEXEC", 0),
            )
            method = "private-copy"
        copy_descriptor(source, descriptor, expected_bytes)
        source_after_copy = os.fstat(source)
        if stat_identity(source_metadata) != stat_identity(source_after_copy):
            raise RuntimeError("binary identity changed while copied for pinning")
        if stat_identity(canonical_before) != stat_identity(canonical.lstat()):
            raise RuntimeError("binary path changed while copied for pinning")
        os.fchmod(descriptor, 0o500)
        os.fsync(descriptor)
        seals = 0
        if method == "linux-sealed-memfd-fexecve":
            seals = (
                fcntl.F_SEAL_SEAL
                | fcntl.F_SEAL_SHRINK
                | fcntl.F_SEAL_GROW
                | fcntl.F_SEAL_WRITE
            )
            fcntl.fcntl(descriptor, fcntl.F_ADD_SEALS, seals)
        metadata = os.fstat(descriptor)
        pinned_checksum = sha256_descriptor(descriptor, expected_bytes)
        metadata_after_hash = os.fstat(descriptor)
        if stat_identity(metadata) != stat_identity(metadata_after_hash):
            raise RuntimeError("pinned executable identity changed while hashed")
        record: dict[str, object] = {
            "method": method,
            "sha256": pinned_checksum,
            "bytes": metadata.st_size,
            "mode": stat.S_IMODE(metadata.st_mode),
            "device": metadata.st_dev,
            "inode": metadata.st_ino,
            "seals": seals,
        }
        pinned = PinnedExecutable(
            descriptor,
            method,
            record,
            execution_path,
            temporary,
        )
        pinned.verify()
        os.close(source)
        source = None
        verify_executable_identity(identity)
        return pinned
    except BaseException:
        if source is not None:
            try:
                os.close(source)
            except BaseException:
                pass
        if pinned is not None:
            try:
                pinned.close()
            except BaseException:
                pass
        elif descriptor is not None:
            try:
                os.close(descriptor)
            except BaseException:
                pass
            if temporary is not None:
                try:
                    temporary.cleanup()
                except BaseException:
                    pass
        elif temporary is not None:
            try:
                temporary.cleanup()
            except BaseException:
                pass
        raise


def run_measurement(
    project: str,
    command: list[str],
    cwd: Path,
    environment: Mapping[str, str],
    warmup: int,
    runs: int,
    timeout_seconds: float,
    expected_executable: Mapping[str, object],
) -> dict[str, object]:
    def run_once() -> dict[str, int]:
        verify_executable_identity(expected_executable)
        pinned.verify()
        try:
            completed = run_process(
                command,
                cwd=cwd,
                environment=environment,
                timeout_seconds=timeout_seconds,
                capture_output=False,
                executable_descriptor=(
                    pinned.descriptor if sys.platform.startswith("linux") else None
                ),
                executable_path=pinned.execution_path,
            )
        finally:
            pinned.verify()
            verify_executable_identity(expected_executable)
        return {
            "elapsed_ns": completed.elapsed_ns,
            "setup_ns": completed.setup_ns,
            "supervision_ns": completed.supervision_ns,
            "cleanup_ns": completed.cleanup_ns,
            "returncode": completed.returncode,
        }

    pinned = pin_executable(expected_executable)
    try:
        for _ in range(warmup):
            sample = run_once()
            if sample["returncode"] != 0:
                raise RuntimeError(f"{project} warmup exited {sample['returncode']}")
        samples = [run_once() for _ in range(runs)]
        failed = [sample for sample in samples if sample["returncode"] != 0]
        if failed:
            raise RuntimeError(f"{project} measured run exited {failed[0]['returncode']}")
        elapsed = [sample["elapsed_ns"] for sample in samples]
        result = {
            "project": project,
            "status": "measured",
            "command": command,
            "cwd": str(cwd),
            "environment": dict(environment),
            "timeout_seconds": timeout_seconds,
            "warmup": warmup,
            "executable_identity": dict(expected_executable),
            "pinned_executable": dict(pinned.record),
            "samples": samples,
            "median_ns": integer_median(elapsed),
            "p95_ns": percentile_95(elapsed),
        }
    except BaseException:
        try:
            pinned.close()
        except BaseException:
            pass
        raise
    pinned.close()
    return result


def isolated_workload_context(
    scratch_dir: Path,
    workload: str,
    project: str,
    base: Mapping[str, str],
) -> tuple[Path, dict[str, str]]:
    root = scratch_dir / "comparison" / workload / project
    workspace = root / "workspace"
    home = workspace / "home" if workload == "status-json" else root / "home"
    temporary = workspace / "tmp" if workload == "status-json" else root / "tmp"
    root.mkdir(parents=True, mode=0o700)
    workspace.mkdir(mode=0o700)
    for directory in (home, temporary):
        directory.mkdir(mode=0o700)
    environment = {**base, "HOME": str(home), "TMPDIR": str(temporary)}
    if workload == "status-json":
        xdg_config = workspace / "xdg-config"
        xdg_config.mkdir(mode=0o700)
        environment.update(
            {
                "AI_GATEWAY_API_KEY": STATUS_JSON_DUMMY_API_KEY,
                "XDG_CONFIG_HOME": str(xdg_config),
            }
        )
    return workspace, environment


def create_status_json_fixtures(
    contexts: Sequence[tuple[Path, Mapping[str, str]]],
) -> dict[Path, bytes]:
    fixtures: dict[Path, bytes] = {}
    for workspace, environment in contexts:
        for path, contents in (
            (Path(environment["HOME"]) / ".fx" / "settings.json", FX_STATUS_CONFIG),
            (
                Path(environment["XDG_CONFIG_HOME"]) / "machine-god" / "config.json",
                MACHINE_STATUS_CONFIG,
            ),
        ):
            if path.exists():
                raise RuntimeError(f"status benchmark fixture already exists: {path}")
            path.parent.mkdir(mode=0o700)
            path.write_bytes(contents)
            path.chmod(0o600)
            if workspace.resolve() not in path.resolve().parents:
                raise RuntimeError("status benchmark fixture escaped its isolated workspace")
            fixtures[path] = contents
    return fixtures


def verify_status_json_fixtures(fixtures: Mapping[Path, bytes]) -> None:
    for path, expected in fixtures.items():
        try:
            metadata = path.stat()
            contents = path.read_bytes()
        except OSError as error:
            raise RuntimeError(f"status benchmark fixture became unreadable: {path}") from error
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_mode & 0o777 != 0o600
            or contents != expected
        ):
            raise RuntimeError(f"status benchmark fixture changed: {path}")


def run_equivalence_probe(
    *,
    method: str,
    commands: Sequence[list[str]],
    contexts: Sequence[tuple[Path, Mapping[str, str]]],
    expected_executables: Sequence[Mapping[str, object]],
    timeout_seconds: float,
    normalizers: Sequence[Callable[[bytes, Path], bytes]],
) -> dict[str, object]:
    if (
        len(commands) != 2
        or len(contexts) != 2
        or len(expected_executables) != 2
        or len(normalizers) != 2
    ):
        raise ValueError("equivalence probes require exactly two implementations")
    records: list[dict[str, object]] = []
    normalized_outputs: list[bytes] = []
    for project, command, (cwd, environment), executable, normalizer in zip(
        ("fx", "machine-god"),
        commands,
        contexts,
        expected_executables,
        normalizers,
        strict=True,
    ):
        completed = run_process(
            command,
            cwd=cwd,
            environment=environment,
            timeout_seconds=timeout_seconds,
            capture_output=True,
            expected_executable=executable,
        )
        if completed.returncode != 0:
            raise RuntimeError(f"{project} equivalence probe exited {completed.returncode}")
        if completed.stderr:
            raise RuntimeError(f"{project} equivalence probe emitted stderr")
        normalized = normalizer(completed.stdout, cwd)
        normalized_outputs.append(normalized)
        records.append(
            {
                "project": project,
                "stdout_sha256": sha256_bytes(completed.stdout),
                "normalized_sha256": sha256_bytes(normalized),
            }
        )
    if normalized_outputs[0] != normalized_outputs[1]:
        raise RuntimeError(f"{method} equivalence probe outputs differ after normalization")
    normalized = normalized_outputs[0]
    substitutions = (
        ["executable-branding"]
        if method == "status-help-executable-brand-v1"
        else ["build-provenance", "isolated-workspace-root"]
    )
    return {
        "method": method,
        "allowed_substitutions": substitutions,
        "normalized_sha256": sha256_bytes(normalized),
        "implementations": records,
    }


def equivalent_workload(
    identifier: str,
    description: str,
    reason: str,
    probe: Mapping[str, object],
    implementations: Sequence[Mapping[str, object]],
) -> dict[str, object]:
    return {
        "id": identifier,
        "description": description,
        "equivalence": "equivalent",
        "claim_eligible": False,
        "reason": reason,
        "equivalence_probe": dict(probe),
        "implementations": [dict(item) for item in implementations],
    }


def unavailable_workloads(
    fx_binary: Path, machine_binary: Path
) -> list[dict[str, object]]:
    implemented = (
        (
            "help",
            [str(fx_binary), "help"],
            [str(machine_binary), "help"],
            "both help commands exist, but their output contracts are not equivalent",
        ),
        (
            "doctor-json",
            [str(fx_binary), "doctor", "--json"],
            [str(machine_binary), "doctor", "--json"],
            (
                "machine-god reports four bounded local readiness checks with "
                "intentional status, output, and exit-semantics differences from fx"
            ),
        ),
        (
            "sessions-json",
            [str(fx_binary), "sessions", "--json"],
            [str(machine_binary), "sessions", "--json"],
            (
                "machine-god reports only bounded persisted session IDs with "
                "intentional schema, metadata, and truncation differences from fx"
            ),
        ),
        (
            "background-json",
            [str(fx_binary), "background", "--json"],
            [str(machine_binary), "background", "--json"],
            (
                "machine-god reports only bounded read-only persisted background "
                "history with intentional schema and lifecycle differences from fx"
            ),
        ),
    )
    implemented_records = [
        {
            "id": identifier,
            "description": (
                f"Pinned local commands: {' '.join(fx_command[1:])} and "
                f"{' '.join(machine_command[1:])}"
            ),
            "equivalence": "non-equivalent",
            "claim_eligible": False,
            "reason": reason,
            "implementations": [
                {
                    "project": "fx",
                    "status": "not-measured",
                    "command": fx_command,
                    "reason": "non-equivalent commands are intentionally not measured",
                },
                {
                    "project": "machine-god",
                    "status": "not-measured",
                    "command": machine_command,
                    "reason": "non-equivalent commands are intentionally not measured",
                },
            ],
        }
        for identifier, fx_command, machine_command, reason in implemented
    ]
    return implemented_records


def base_environment(home: Path, temporary: Path, containment_token: str) -> dict[str, str]:
    path_value = os.environ.get("PATH")
    if not path_value:
        raise RuntimeError("PATH is required to resolve pinned build tools")
    return {
        "HOME": str(home),
        "LANG": "C",
        "LC_ALL": "C",
        CONTAINMENT_ENVIRONMENT_KEY: containment_token,
        "NO_COLOR": "1",
        "PATH": path_value,
        "TMPDIR": str(temporary),
    }


def cpu_model() -> str:
    if platform.system() == "Linux":
        try:
            for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines():
                if line.lower().startswith("model name"):
                    return line.split(":", 1)[1].strip()
        except OSError:
            pass
    if platform.system() == "Darwin":
        for name in ("machdep.cpu.brand_string", "hw.model"):
            try:
                value = subprocess.check_output(
                    ["sysctl", "-n", name], text=True, timeout=2
                ).strip()
            except (OSError, subprocess.SubprocessError):
                continue
            if value:
                return value
    return platform.processor() or "unknown"


def runner_record(runner_class: str) -> dict[str, object]:
    return {
        "class": runner_class,
        "github_actions": os.environ.get("GITHUB_ACTIONS") == "true",
        "image_os": os.environ.get("ImageOS", "local"),
        "image_version": os.environ.get("ImageVersion", "local"),
        "runner_os": os.environ.get("RUNNER_OS", platform.system()),
        "runner_arch": os.environ.get("RUNNER_ARCH", platform.machine()),
    }


def collect_evidence(args: argparse.Namespace) -> dict[str, object]:
    root = args.root.resolve()
    lock_path = args.lock.resolve()
    upstream_dir = args.upstream_dir.resolve()
    scratch_dir = args.scratch_dir.resolve()
    lock = parse_upstream_lock(lock_path)
    if args.runs < 10 or args.warmup < 1:
        raise ValueError("runs must be >= 10 and warmup must be >= 1")
    for name in ("fetch_timeout", "build_timeout", "sample_timeout"):
        if not is_positive_number(getattr(args, name)):
            raise ValueError(f"{name.replace('_', '-')} must be a positive finite number")
    require_text(args.runner_class, "runner_class")
    if upstream_dir.exists():
        raise RuntimeError(f"fresh upstream checkout path already exists: {upstream_dir}")
    if scratch_dir.exists():
        raise RuntimeError(f"fresh scratch path already exists: {scratch_dir}")

    scratch_dir.mkdir(parents=True, mode=0o700)
    home = scratch_dir / "home"
    temporary = scratch_dir / "tmp"
    home.mkdir(mode=0o700)
    temporary.mkdir(mode=0o700)
    base_env = base_environment(home, temporary, secrets.token_hex(16))
    cargo_home = scratch_dir / "cargo-home"
    cargo_home.mkdir(mode=0o700)
    rustup_home = Path(os.environ.get("RUSTUP_HOME", Path.home() / ".rustup")).resolve()
    tool_env = {
        **base_env,
        "CARGO_HOME": str(cargo_home),
        "RUSTUP_HOME": str(rustup_home),
    }
    git_env = {
        **base_env,
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_NO_REPLACE_OBJECTS": "1",
        "GIT_TERMINAL_PROMPT": "0",
    }

    path_value = base_env["PATH"]
    git = invocation_path(args.git, path_value)
    zig = invocation_path(args.zig, path_value)
    rustc = invocation_path(args.rustc, path_value)
    cargo = invocation_path(args.cargo, path_value)
    tools = verify_tool_versions(
        git,
        zig,
        rustc,
        cargo,
        environment=tool_env,
        timeout_seconds=args.fetch_timeout,
    )
    git_tool = tools["git"]
    check_machine_cleanliness(
        root,
        git,
        git_env,
        args.fetch_timeout,
        expected_executable=git_tool,
    )
    machine_sha = git_output(
        git,
        root,
        git_env,
        args.fetch_timeout,
        "rev-parse",
        "HEAD",
        expected_executable=git_tool,
    )
    if not HEX_SHA_RE.fullmatch(machine_sha):
        raise RuntimeError(f"machine-god HEAD is not a full Git SHA: {machine_sha}")
    machine_source_dir = scratch_dir / "machine-source"
    machine_manifest_path = scratch_dir / "machine-source-manifest.json"
    machine_materialization = materialize_machine_source(
        root,
        machine_source_dir,
        machine_manifest_path,
        machine_sha,
        git,
        environment=git_env,
        timeout_seconds=args.fetch_timeout,
        expected_executable=git_tool,
    )

    plan = command_plan(
        root,
        upstream_dir,
        lock,
        git=git,
        zig=zig,
        cargo=cargo,
    )
    verified_commit, preparation = prepare_upstream(
        root,
        upstream_dir,
        lock,
        plan,
        git,
        environment=git_env,
        timeout_seconds=args.fetch_timeout,
        expected_executable=git_tool,
    )

    fx_environment = {
        **base_env,
        "ZIG_GLOBAL_CACHE_DIR": str(scratch_dir / "zig-global-cache"),
        "ZIG_LOCAL_CACHE_DIR": str(upstream_dir / ".zig-cache"),
    }
    machine_target = scratch_dir / "machine-target"
    machine_environment = {
        **base_env,
        "CARGO_HOME": str(cargo_home),
        "CARGO_INCREMENTAL": "0",
        "CARGO_TARGET_DIR": str(machine_target),
        "RUSTUP_HOME": str(rustup_home),
    }
    fx_build = run_record(
        plan["fx_build"],
        upstream_dir,
        environment=fx_environment,
        timeout_seconds=args.build_timeout,
        expected_executable=tools["zig"],
    )
    fx_build.update(
        {
            "project": "fx",
            "profile": "ReleaseSafe",
            "binary": binary_record(upstream_dir / "zig-out/bin/fx"),
        }
    )
    machine_build = run_record(
        plan["machine_god_build"],
        machine_source_dir,
        environment=machine_environment,
        timeout_seconds=args.build_timeout,
        expected_executable=tools["cargo"],
    )
    machine_build.update(
        {
            "project": "machine-god",
            "profile": "release",
            "binary": binary_record(machine_target / "release/machine-god"),
        }
    )

    fx_binary = Path(str(fx_build["binary"]["path"]))
    machine_binary = Path(str(machine_build["binary"]["path"]))
    fx_executable_identity = executable_identity(fx_binary)
    machine_executable_identity = executable_identity(machine_binary)
    fx_measurement_environment = {**base_env, "FX_BENCH": "1"}
    bootstrap = {
        "id": "bootstrap-exit",
        "description": BOOTSTRAP_DESCRIPTION,
        "equivalence": "non-equivalent",
        "claim_eligible": False,
        "reason": BOOTSTRAP_REASON,
        "implementations": [
            run_measurement(
                "fx",
                [str(fx_binary)],
                machine_source_dir,
                fx_measurement_environment,
                args.warmup,
                args.runs,
                args.sample_timeout,
                fx_executable_identity,
            ),
            run_measurement(
                "machine-god",
                [str(machine_binary)],
                machine_source_dir,
                base_env,
                args.warmup,
                args.runs,
                args.sample_timeout,
                machine_executable_identity,
            ),
        ],
    }
    status_help_contexts = [
        isolated_workload_context(scratch_dir, "status-help", project, base_env)
        for project in ("fx", "machine-god")
    ]
    status_help_commands = (
        [str(fx_binary), "status", "--help"],
        [str(machine_binary), "status", "--help"],
    )
    status_help_probe = run_equivalence_probe(
        method="status-help-executable-brand-v1",
        commands=status_help_commands,
        contexts=status_help_contexts,
        expected_executables=(
            fx_executable_identity,
            machine_executable_identity,
        ),
        timeout_seconds=args.sample_timeout,
        normalizers=(
            lambda output, _workspace: normalize_status_help(output),
            lambda output, _workspace: normalize_status_help(output),
        ),
    )
    status_help = equivalent_workload(
        "status-help",
        STATUS_HELP_DESCRIPTION,
        STATUS_HELP_REASON,
        status_help_probe,
        [
            run_measurement(
                project,
                command,
                context[0],
                context[1],
                args.warmup,
                args.runs,
                args.sample_timeout,
                identity,
            )
            for project, command, context, identity in zip(
                ("fx", "machine-god"),
                status_help_commands,
                status_help_contexts,
                (fx_executable_identity, machine_executable_identity),
                strict=True,
            )
        ],
    )
    status_json_contexts = [
        isolated_workload_context(scratch_dir, "status-json", project, base_env)
        for project in ("fx", "machine-god")
    ]
    status_json_fixtures = create_status_json_fixtures(status_json_contexts)
    status_json_commands = (
        [str(fx_binary), "status", "--json"],
        [str(machine_binary), "status", "--json"],
    )
    status_json_probe = run_equivalence_probe(
        method="status-json-runtime-schema-v1",
        commands=status_json_commands,
        contexts=status_json_contexts,
        expected_executables=(
            fx_executable_identity,
            machine_executable_identity,
        ),
        timeout_seconds=args.sample_timeout,
        normalizers=(
            lambda output, workspace: normalize_status_json(
                output, workspace, lock.commit[:12]
            ),
            lambda output, workspace: normalize_status_json(output, workspace, ""),
        ),
    )
    status_json_probe["fixture_sha256"] = {
        "fx_settings": sha256_bytes(FX_STATUS_CONFIG),
        "machine_god_config": sha256_bytes(MACHINE_STATUS_CONFIG),
    }
    status_json = equivalent_workload(
        "status-json",
        STATUS_JSON_DESCRIPTION,
        STATUS_JSON_REASON,
        status_json_probe,
        [
            run_measurement(
                project,
                command,
                context[0],
                context[1],
                args.warmup,
                args.runs,
                args.sample_timeout,
                identity,
            )
            for project, command, context, identity in zip(
                ("fx", "machine-god"),
                status_json_commands,
                status_json_contexts,
                (fx_executable_identity, machine_executable_identity),
                strict=True,
            )
        ],
    )
    verify_status_json_fixtures(status_json_fixtures)
    unavailable = unavailable_workloads(fx_binary, machine_binary)
    if (
        verify_materialized_source(
            machine_source_dir, machine_materialization["entries"]
        )
        != machine_materialization["source_tree_sha256"]
    ):
        raise RuntimeError("materialized machine-god source changed during build or measurement")

    evidence = {
        "schema_version": 2,
        "classification": "mixed-pinned-comparison-evidence",
        "claim_eligible": False,
        "generated_at_utc": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "runner_class": args.runner_class,
        "timeouts_seconds": {
            "fetch": args.fetch_timeout,
            "build": args.build_timeout,
            "sample": args.sample_timeout,
        },
        "source": {
            "machine_god": {
                "git_sha": machine_sha,
                "dirty": False,
                "repository_root": str(root),
                "allowed_output_directories": list(ALLOWED_MACHINE_OUTPUTS),
                "materialization": machine_materialization,
            },
            "fx": {
                "repository": lock.repository,
                "locked_commit": lock.commit,
                "verified_commit": verified_commit,
                "lock_path": str(lock_path),
                "lock_sha256": sha256_file(lock_path),
                "fresh_checkout": True,
                "hooks_disabled": True,
                "preparation_commands": preparation,
            },
        },
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": platform.python_version(),
            "cpu_count": os.cpu_count() or 1,
            "cpu_model": cpu_model(),
            "runner": runner_record(args.runner_class),
        },
        "tools": tools,
        "tool_environment": tool_env,
        "builds": [fx_build, machine_build],
        "environment_policy": {
            "inherits_parent_environment": False,
            "allowlisted_environment_only": True,
        },
        "workloads": [
            bootstrap,
            unavailable[0],
            status_help,
            status_json,
            *unavailable[1:],
        ],
    }
    validate_upstream_evidence(
        evidence,
        expected_lock=lock,
        expected_lock_path=lock_path,
        expected_lock_sha256=sha256_file(lock_path),
        expected_root=root,
        expected_runner_class=args.runner_class,
        expected_machine_tree=str(machine_materialization["git_tree"]),
        expected_machine_manifest_sha256=str(
            machine_materialization["git_entries_sha256"]
        ),
    )
    return evidence


def write_evidence_atomic(output: Path, evidence: Mapping[str, Any]) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{output.name}.",
        suffix=".partial",
        dir=output.parent,
    )
    temporary = Path(temporary_name)
    created = os.fstat(descriptor)
    published = False
    try:
        if not stat.S_ISREG(created.st_mode) or created.st_nlink != 1:
            raise RuntimeError("exclusive evidence temporary is not a regular file")
        with os.fdopen(os.dup(descriptor), "w", encoding="utf-8") as destination:
            destination.write(json.dumps(evidence, indent=2) + "\n")
            destination.flush()
            os.fsync(destination.fileno())
        current = os.stat(temporary, follow_symlinks=False)
        if (
            not stat.S_ISREG(current.st_mode)
            or current.st_dev != created.st_dev
            or current.st_ino != created.st_ino
            or current.st_nlink != 1
        ):
            raise RuntimeError("exclusive evidence temporary identity changed")
        os.replace(temporary, output)
        published = True
        installed = os.stat(output, follow_symlinks=False)
        if (
            not stat.S_ISREG(installed.st_mode)
            or installed.st_dev != created.st_dev
            or installed.st_ino != created.st_ino
        ):
            raise RuntimeError("published evidence is not the verified regular file")
        directory_descriptor = os.open(
            output.parent,
            os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
        )
        try:
            os.fsync(directory_descriptor)
        finally:
            os.close(directory_descriptor)
    finally:
        os.close(descriptor)
        if not published:
            try:
                leftover = os.stat(temporary, follow_symlinks=False)
                if leftover.st_dev == created.st_dev and leftover.st_ino == created.st_ino:
                    temporary.unlink()
            except OSError:
                pass


def acquire_output_lock(output: Path, timeout_seconds: float = 1.0) -> OutputLock:
    """Acquire a bounded, exclusive full-run lock without deleting stale locks."""

    if not is_positive_number(timeout_seconds):
        raise ValueError("output lock timeout must be a positive finite number")
    output.parent.mkdir(parents=True, exist_ok=True)
    lock_path = output.with_name(f".{output.name}.lock")
    deadline = time.monotonic() + timeout_seconds
    flags = (
        os.O_RDWR
        | os.O_CREAT
        | os.O_EXCL
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NOFOLLOW", 0)
    )
    while True:
        try:
            descriptor = os.open(lock_path, flags, 0o600)
            break
        except FileExistsError as error:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RuntimeError(
                    f"evidence output is locked by another invocation: {lock_path}"
                ) from error
            time.sleep(min(0.05, remaining))
    created = os.fstat(descriptor)
    try:
        current = os.stat(lock_path, follow_symlinks=False)
        if (
            not stat.S_ISREG(created.st_mode)
            or created.st_nlink != 1
            or not stat.S_ISREG(current.st_mode)
            or current.st_dev != created.st_dev
            or current.st_ino != created.st_ino
        ):
            raise RuntimeError("exclusive evidence lock identity is invalid")
        with os.fdopen(os.dup(descriptor), "w", encoding="utf-8") as lock_file:
            lock_file.write(
                json.dumps(
                    {"pid": os.getpid(), "token": secrets.token_hex(16)},
                    sort_keys=True,
                )
                + "\n"
            )
            lock_file.flush()
            os.fsync(lock_file.fileno())
        return OutputLock(lock_path, descriptor, created.st_dev, created.st_ino)
    except BaseException:
        os.close(descriptor)
        try:
            leftover = os.stat(lock_path, follow_symlinks=False)
            if leftover.st_dev == created.st_dev and leftover.st_ino == created.st_ino:
                lock_path.unlink()
        except OSError:
            pass
        raise


def release_output_lock(lock: OutputLock) -> None:
    """Release only the lock inode created by this invocation."""

    try:
        try:
            current = os.stat(lock.path, follow_symlinks=False)
            if current.st_dev == lock.device and current.st_ino == lock.inode:
                lock.path.unlink()
                directory_descriptor = os.open(
                    lock.path.parent,
                    os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
                )
                try:
                    os.fsync(directory_descriptor)
                finally:
                    os.close(directory_descriptor)
        except OSError:
            pass
    finally:
        os.close(lock.descriptor)


def collect_and_publish_evidence(
    output: Path,
    producer: Callable[[], Mapping[str, Any]],
    *,
    lock_timeout_seconds: float = 1.0,
) -> Mapping[str, Any]:
    lock = acquire_output_lock(output, lock_timeout_seconds)
    try:
        evidence = producer()
        write_evidence_atomic(output, evidence)
    finally:
        release_output_lock(lock)
    return evidence


def main() -> int:
    default_root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--root", type=Path, default=default_root)
    parser.add_argument("--lock", type=Path, default=default_root / "benchmarks/upstream.lock")
    parser.add_argument("--upstream-dir", type=Path, default=default_root / ".bench/fx")
    parser.add_argument("--scratch-dir", type=Path, default=default_root / ".bench/scratch")
    parser.add_argument(
        "--output",
        type=Path,
        default=default_root / "benchmarks/results/upstream-bootstrap.json",
    )
    parser.add_argument("--runner-class", default=f"local-{platform.system()}-{platform.machine()}")
    parser.add_argument("--runs", type=int, default=30)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--fetch-timeout", type=float, default=300.0)
    parser.add_argument("--build-timeout", type=float, default=1200.0)
    parser.add_argument("--sample-timeout", type=float, default=10.0)
    parser.add_argument("--git", default="git")
    parser.add_argument("--zig", default="zig")
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--cargo", default="cargo")
    args = parser.parse_args()

    requested_output = args.output.absolute()
    output = requested_output.parent.resolve() / requested_output.name
    try:
        with termination_signal_handlers():
            collect_and_publish_evidence(output, lambda: collect_evidence(args))
    except HarnessSignal as caught:
        return 128 + caught.signum
    except (OSError, subprocess.SubprocessError, RuntimeError, ValueError) as error:
        parser.exit(1, f"error: {error}\n")
    print(f"wrote validated bootstrap evidence to {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
