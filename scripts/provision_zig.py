"""Provision the exact Zig toolchain used to build the pinned fx benchmark.

The Rust product does not use Zig. Callers receive a fresh checksum-verified
extraction only for the lifetime of the ``provisioned_zig`` context.
"""

from __future__ import annotations

import bisect
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import secrets
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from typing import Callable, Iterator


ZIG_VERSION = "0.16.0"
DOWNLOAD_BASE = f"https://ziglang.org/download/{ZIG_VERSION}"
MARKER_NAME = ".machine-god-zig.json"
DOWNLOAD_TIMEOUT_SECONDS = 300
DOWNLOAD_RETRIES = 3
CACHE_LOCK_NAME = ".machine-god-zig.lock"
ACTIVE_LOCK_NAME = ".machine-god-zig-active.lock"
LEASE_NAME = ".machine-god-zig.lease"
TRASH_PREFIX = ".machine-god-zig-trash-"
TRASH_LEASE_SUFFIX = ".lease"
TRASH_CLEANUP_BATCH_SIZE = 8
TRASH_SCAN_LIMIT = 64
TRASH_CURSOR_NAME = ".machine-god-zig-trash.cursor"
ACTIVE_CURSOR_NAME = ".machine-god-zig-active.cursor"
STALE_ACTIVE_BATCH_SIZE = 8
STALE_ACTIVE_SCAN_LIMIT = 64
ACTIVE_DIRECTORY_CAPACITY = 128
CACHE_FIXED_ENTRY_RESERVE = 32
CACHE_DIRECTORY_CAPACITY = (
    CACHE_FIXED_ENTRY_RESERVE + 2 * ACTIVE_DIRECTORY_CAPACITY
)
ARCHIVE_DIRECTORY_CAPACITY = 128
ACTIVE_DIRECTORY_SCAN_CAP = ACTIVE_DIRECTORY_CAPACITY + 1
CACHE_DIRECTORY_SCAN_CAP = CACHE_DIRECTORY_CAPACITY + 1
ARCHIVE_DIRECTORY_SCAN_CAP = ARCHIVE_DIRECTORY_CAPACITY + 1
PARTIAL_ARCHIVE_PRUNE_BATCH_SIZE = 16
RECOVERY_ADMISSION_ATTEMPTS = 2
CACHE_LOCK_TIMEOUT_SECONDS = 30.0
CACHE_LOCK_RETRY_SECONDS = 0.05
DEFERRED_SIGNALS = tuple(
    candidate
    for candidate in (
        getattr(signal, "SIGHUP", None),
        signal.SIGINT,
        signal.SIGTERM,
    )
    if candidate is not None
)

CommandRunner = Callable[..., subprocess.CompletedProcess]


@dataclass(frozen=True)
class ToolchainSpec:
    target: str
    sha256: str
    size: int

    @property
    def archive_name(self) -> str:
        return f"zig-{self.target}-{ZIG_VERSION}.tar.xz"

    @property
    def url(self) -> str:
        return f"{DOWNLOAD_BASE}/{self.archive_name}"


@dataclass(frozen=True)
class PrivateTrash:
    path: Path
    lease_path: Path
    descriptor: int


TOOLCHAINS = {
    ("Darwin", "arm64"): ToolchainSpec(
        "aarch64-macos",
        "b23d70deaa879b5c2d486ed3316f7eaa53e84acf6fc9cc747de152450d401489",
        52_238_004,
    ),
    ("Darwin", "x86_64"): ToolchainSpec(
        "x86_64-macos",
        "0387557ed1877bc6a2e1802c8391953baddba76081876301c522f52977b52ba7",
        57_396_836,
    ),
    ("Linux", "aarch64"): ToolchainSpec(
        "aarch64-linux",
        "ea4b09bfb22ec6f6c6ceac57ab63efb6b46e17ab08d21f69f3a48b38e1534f17",
        51_211_944,
    ),
    ("Linux", "x86_64"): ToolchainSpec(
        "x86_64-linux",
        "70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00",
        55_478_392,
    ),
}


class ProvisionError(RuntimeError):
    """A bounded provisioning failure."""


def host_spec(system: str | None = None, machine: str | None = None) -> ToolchainSpec:
    system = platform.system() if system is None else system
    machine = platform.machine() if machine is None else machine
    machine = {"AMD64": "x86_64", "arm64": "arm64"}.get(machine, machine)
    try:
        return TOOLCHAINS[(system, machine)]
    except KeyError as error:
        raise ProvisionError(
            f"Zig {ZIG_VERSION} provisioning is unsupported on {system}/{machine}"
        ) from error


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def executable_version(
    executable: Path, run: CommandRunner = subprocess.run
) -> str | None:
    try:
        completed = run(
            [str(executable), "version"],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if completed.returncode != 0:
        return None
    return completed.stdout.strip()


def marker_payload(spec: ToolchainSpec) -> dict[str, object]:
    return {
        "archive": spec.archive_name,
        "archive_sha256": spec.sha256,
        "archive_size": spec.size,
        "target": spec.target,
        "version": ZIG_VERSION,
    }


def validated_archive(archive: Path, spec: ToolchainSpec) -> Path | None:
    try:
        archive_status = archive.stat(follow_symlinks=False)
    except OSError:
        return None
    if not stat.S_ISREG(archive_status.st_mode):
        return None
    if archive_status.st_size != spec.size:
        return None
    if file_sha256(archive) != spec.sha256:
        return None
    return archive.resolve(strict=True)


def download_archive(
    destination: Path, spec: ToolchainSpec, run: CommandRunner = subprocess.run
) -> None:
    curl = shutil.which("curl")
    if curl is None:
        raise ProvisionError("curl is required to provision the pinned Zig toolchain")
    command = [
        curl,
        "--fail",
        "--show-error",
        "--silent",
        "--location",
        "--max-redirs",
        "3",
        "--proto",
        "=https",
        "--tlsv1.2",
        "--connect-timeout",
        "30",
        "--max-time",
        str(DOWNLOAD_TIMEOUT_SECONDS),
        "--speed-limit",
        "1024",
        "--speed-time",
        "60",
        "--retry",
        str(DOWNLOAD_RETRIES),
        "--retry-all-errors",
        "--retry-delay",
        "2",
        "--retry-max-time",
        "900",
        "--max-filesize",
        str(spec.size),
        "--output",
        str(destination),
        spec.url,
    ]
    try:
        completed = run(command, check=False, timeout=930)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ProvisionError(f"could not download {spec.url}: {error}") from error
    if completed.returncode != 0:
        raise ProvisionError(f"could not download {spec.url}")
    actual_size = destination.stat().st_size
    if actual_size != spec.size:
        raise ProvisionError(
            f"downloaded Zig archive size is {actual_size}, expected {spec.size}"
        )
    actual_sha256 = file_sha256(destination)
    if actual_sha256 != spec.sha256:
        raise ProvisionError(
            f"downloaded Zig archive SHA-256 is {actual_sha256}, expected {spec.sha256}"
        )


def extract_archive(
    archive: Path, destination: Path, run: CommandRunner = subprocess.run
) -> None:
    tar = shutil.which("tar")
    if tar is None:
        raise ProvisionError("tar with xz support is required to provision Zig")
    destination.mkdir(mode=0o700)
    command = [
        tar,
        "--extract",
        "--xz",
        "--file",
        str(archive),
        "--directory",
        str(destination),
        "--strip-components=1",
    ]
    try:
        completed = run(command, check=False, timeout=120)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ProvisionError(f"could not extract the Zig archive: {error}") from error
    if completed.returncode != 0:
        raise ProvisionError("could not extract the Zig archive")


def write_marker(install_dir: Path, spec: ToolchainSpec) -> None:
    (install_dir / MARKER_NAME).write_text(
        json.dumps(marker_payload(spec), sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )


def ensure_archive(
    install_root: Path,
    spec: ToolchainSpec,
    run: CommandRunner = subprocess.run,
    *,
    known_archive_entries: int | None = None,
) -> Path:
    archive_root = install_root / "archives"
    archive_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    archive = archive_root / spec.archive_name
    cached = validated_archive(archive, spec)
    if cached is not None:
        return cached
    if archive.exists() or archive.is_symlink():
        raise ProvisionError(
            f"cached Zig archive is invalid; move it aside before retrying: {archive}"
        )
    if known_archive_entries is None:
        known_archive_entries = len(
            bounded_directory_names(
                archive_root,
                ARCHIVE_DIRECTORY_SCAN_CAP,
                "Zig archive cache",
            )
        )
    if known_archive_entries + 2 > ARCHIVE_DIRECTORY_CAPACITY:
        raise ProvisionError(
            "Zig archive cache has no capacity for temporary publication"
        )

    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{spec.archive_name}.", dir=archive_root
    )
    os.close(descriptor)
    temporary = Path(temporary_name)
    try:
        download_archive(temporary, spec, run)
        try:
            os.link(temporary, archive)
        except FileExistsError:
            cached = validated_archive(archive, spec)
            if cached is None:
                raise ProvisionError(f"concurrent Zig archive is invalid: {archive}")
            return cached
        cached = validated_archive(archive, spec)
        if cached is None:
            raise ProvisionError("published Zig archive failed validation")
        return cached
    finally:
        temporary.unlink(missing_ok=True)


def open_private_file(path: Path, *, exclusive: bool = False) -> int:
    flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    if exclusive:
        flags |= os.O_EXCL
    descriptor = os.open(path, flags, 0o600)
    try:
        os.fchmod(descriptor, 0o600)
    except BaseException:
        try:
            os.close(descriptor)
        except OSError:
            pass
        raise
    return descriptor


def open_existing_private_file(path: Path) -> int:
    flags = os.O_RDWR | getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    return os.open(path, flags)


def bounded_directory_names(root: Path, scan_cap: int, label: str) -> list[str]:
    """Materialize fewer than ``scan_cap`` names or fail on the cap witness."""

    names: list[str] = []
    with os.scandir(root) as entries:
        for entry in entries:
            if len(names) + 1 >= scan_cap:
                raise ProvisionError(
                    f"{label} exceeds the bounded {scan_cap - 1}-entry limit"
                )
            names.append(entry.name)
    return names


@contextmanager
def defer_termination_signals() -> Iterator[None]:
    """Defer wrapper termination until an owned resource has a cleanup guard."""

    previous = signal.pthread_sigmask(signal.SIG_BLOCK, DEFERRED_SIGNALS)
    try:
        yield
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous)


@contextmanager
def cache_lock(cache_root: Path, name: str = CACHE_LOCK_NAME) -> Iterator[None]:
    descriptor = open_private_file(cache_root / name)
    deadline = time.monotonic() + CACHE_LOCK_TIMEOUT_SECONDS
    try:
        while True:
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise ProvisionError("timed out waiting for the Zig cache lock") from None
                time.sleep(CACHE_LOCK_RETRY_SECONDS)
        yield
    finally:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_UN)
        finally:
            os.close(descriptor)


def prune_partial_archives(cache_root: Path, spec: ToolchainSpec) -> int:
    archive_root = cache_root / "archives"
    archive_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    prefix = f".{spec.archive_name}."
    names = bounded_directory_names(
        archive_root, ARCHIVE_DIRECTORY_SCAN_CAP, "Zig archive cache"
    )
    pruned = 0
    for name in names:
        if pruned >= PARTIAL_ARCHIVE_PRUNE_BATCH_SIZE:
            break
        candidate = archive_root / name
        if not candidate.name.startswith(prefix):
            continue
        try:
            status = candidate.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISDIR(status.st_mode):
            raise ProvisionError("unexpected Zig archive temporary directory")
        candidate.unlink()
        pruned += 1
    return len(names) - pruned


def trash_lease_path(trash_path: Path) -> Path:
    return trash_path.with_name(f"{trash_path.name}{TRASH_LEASE_SUFFIX}")


def create_private_trash(cache_root: Path) -> PrivateTrash:
    """Create and exclusively lease an unguessable same-filesystem trash root."""

    for _attempt in range(16):
        path = cache_root / f"{TRASH_PREFIX}{secrets.token_hex(16)}"
        lease_path = trash_lease_path(path)
        try:
            descriptor = open_private_file(lease_path, exclusive=True)
        except FileExistsError:
            continue
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            path.mkdir(mode=0o700)
            return PrivateTrash(path, lease_path, descriptor)
        except FileExistsError:
            os.close(descriptor)
            lease_path.unlink(missing_ok=True)
            continue
        except BaseException:
            os.close(descriptor)
            lease_path.unlink(missing_ok=True)
            shutil.rmtree(path, ignore_errors=True)
            raise
    raise ProvisionError("could not allocate unique Zig trash ownership")


def claim_abandoned_trash(
    cache_root: Path,
    candidate_names: list[str],
    *,
    batch_size: int = TRASH_CLEANUP_BATCH_SIZE,
) -> list[PrivateTrash]:
    """Exclusively claim bounded abandoned trash while the active lock is held."""

    claimed: list[PrivateTrash] = []
    try:
        for name in candidate_names:
            if len(claimed) >= batch_size:
                break
            if not name.startswith(TRASH_PREFIX) or name.endswith(
                TRASH_LEASE_SUFFIX
            ):
                continue
            path = cache_root / name
            try:
                status = path.lstat()
            except FileNotFoundError:
                continue
            if not stat.S_ISDIR(status.st_mode) or stat.S_ISLNK(status.st_mode):
                raise ProvisionError("unexpected Zig trash-cache entry")
            lease_path = trash_lease_path(path)
            try:
                descriptor = open_existing_private_file(lease_path)
            except FileNotFoundError:
                if not path.exists() and not path.is_symlink():
                    continue
                raise ProvisionError("Zig trash-cache entry is missing its lease")
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                os.close(descriptor)
                continue
            except BaseException:
                os.close(descriptor)
                raise
            try:
                lease_status = lease_path.lstat()
            except FileNotFoundError:
                os.close(descriptor)
                continue
            descriptor_status = os.fstat(descriptor)
            if (
                lease_status.st_dev != descriptor_status.st_dev
                or lease_status.st_ino != descriptor_status.st_ino
            ):
                os.close(descriptor)
                raise ProvisionError("Zig trash lease changed during claim")
            try:
                current = path.lstat()
            except FileNotFoundError:
                os.close(descriptor)
                continue
            if (
                current.st_dev != status.st_dev
                or current.st_ino != status.st_ino
                or not stat.S_ISDIR(current.st_mode)
                or stat.S_ISLNK(current.st_mode)
            ):
                os.close(descriptor)
                raise ProvisionError("Zig trash-cache entry changed during claim")
            claimed.append(PrivateTrash(path, lease_path, descriptor))
    except BaseException:
        for trash in claimed:
            os.close(trash.descriptor)
        raise
    return claimed


def retire_orphan_trash_leases(
    cache_root: Path,
    candidate_names: list[str],
    *,
    batch_size: int = TRASH_CLEANUP_BATCH_SIZE,
) -> int:
    """Boundedly remove unlocked trash leases whose corresponding tree is absent."""

    retired = 0
    for name in candidate_names:
        if retired >= batch_size:
            break
        if not name.startswith(TRASH_PREFIX) or not name.endswith(
            TRASH_LEASE_SUFFIX
        ):
            continue
        lease_path = cache_root / name
        trash_path = cache_root / name[: -len(TRASH_LEASE_SUFFIX)]
        if trash_path.exists() or trash_path.is_symlink():
            continue
        try:
            descriptor = open_existing_private_file(lease_path)
        except FileNotFoundError:
            continue
        try:
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                continue
            if trash_path.exists() or trash_path.is_symlink():
                continue
            try:
                lease_status = lease_path.lstat()
            except FileNotFoundError:
                continue
            descriptor_status = os.fstat(descriptor)
            if (
                lease_status.st_dev != descriptor_status.st_dev
                or lease_status.st_ino != descriptor_status.st_ino
            ):
                raise ProvisionError("Zig trash lease changed during retirement")
            lease_path.unlink()
            retired += 1
        finally:
            os.close(descriptor)
    return retired


def remove_private_trash(trash: PrivateTrash) -> None:
    """Delete one exclusively leased trash tree and retire its sibling lease."""

    removed = False
    try:
        shutil.rmtree(trash.path)
        removed = True
    finally:
        try:
            if removed:
                with cache_lock(trash.path.parent, ACTIVE_LOCK_NAME):
                    with defer_termination_signals():
                        lease_status = trash.lease_path.lstat()
                        descriptor_status = os.fstat(trash.descriptor)
                        if (
                            lease_status.st_dev != descriptor_status.st_dev
                            or lease_status.st_ino != descriptor_status.st_ino
                        ):
                            raise ProvisionError(
                                "Zig trash lease changed before retirement"
                            )
                        trash.lease_path.unlink()
        finally:
            os.close(trash.descriptor)


def read_scan_cursor(cache_root: Path, name: str) -> str:
    descriptor = open_private_file(cache_root / name)
    try:
        os.lseek(descriptor, 0, os.SEEK_SET)
        payload = os.read(descriptor, 512)
    finally:
        os.close(descriptor)
    try:
        return payload.decode("utf-8")
    except UnicodeDecodeError:
        return ""


def write_scan_cursor(cache_root: Path, name: str, cursor: str) -> None:
    descriptor = open_private_file(cache_root / name)
    try:
        payload = cursor.encode("utf-8")[:511]
        os.ftruncate(descriptor, 0)
        os.lseek(descriptor, 0, os.SEEK_SET)
        while payload:
            written = os.write(descriptor, payload)
            if written <= 0:
                raise OSError("could not update the Zig active-cache cursor")
            payload = payload[written:]
    finally:
        os.close(descriptor)


def rotating_active_window(
    cache_root: Path,
    candidate_names: list[str],
    *,
    scan_limit: int = STALE_ACTIVE_SCAN_LIMIT,
) -> list[str]:
    """Return a persistent round-robin window while the active lock is held."""

    names = candidate_names
    if not names or scan_limit <= 0:
        return []
    cursor = read_scan_cursor(cache_root, ACTIVE_CURSOR_NAME)
    start = bisect.bisect_right(names, cursor)
    rotated = names[start:] + names[:start]
    selected = rotated[:scan_limit]
    write_scan_cursor(cache_root, ACTIVE_CURSOR_NAME, selected[-1])
    return selected


def rotating_trash_window(
    cache_root: Path,
    candidate_names: list[str],
    *,
    scan_limit: int = TRASH_SCAN_LIMIT,
) -> list[str]:
    """Return a persistent round-robin abandoned-trash inspection window."""

    names = candidate_names
    if not names or scan_limit <= 0:
        return []
    cursor = read_scan_cursor(cache_root, TRASH_CURSOR_NAME)
    start = bisect.bisect_right(names, cursor)
    rotated = names[start:] + names[:start]
    selected = rotated[:scan_limit]
    write_scan_cursor(cache_root, TRASH_CURSOR_NAME, selected[-1])
    return selected


def move_stale_active_to_trash(
    active_root: Path,
    trash_root: Path,
    *,
    candidate_names: list[str] | None = None,
    batch_size: int = STALE_ACTIVE_BATCH_SIZE,
    scan_limit: int = STALE_ACTIVE_SCAN_LIMIT,
) -> list[Path]:
    """Detach a bounded number of exclusively leased stale extractions.

    The caller holds ``ACTIVE_LOCK_NAME``. Recursive deletion is deliberately
    left to the caller after that coordination lock has been released.
    """

    moved: list[Path] = []
    scanned = 0
    if candidate_names is None:
        names = bounded_directory_names(
            active_root, ACTIVE_DIRECTORY_SCAN_CAP, "Zig active cache"
        )
    else:
        names = candidate_names
    for name in names:
        if scanned >= scan_limit or len(moved) >= batch_size:
            break
        scanned += 1
        candidate = active_root / name
        if not candidate.name.startswith("zig-"):
            continue
        try:
            status = candidate.lstat()
        except FileNotFoundError:
            continue
        if not stat.S_ISDIR(status.st_mode) or stat.S_ISLNK(status.st_mode):
            raise ProvisionError("unexpected Zig active-cache entry")
        lease_path = candidate / LEASE_NAME
        try:
            descriptor = open_private_file(lease_path)
        except FileNotFoundError:
            continue
        try:
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                continue
            try:
                current = candidate.lstat()
            except FileNotFoundError:
                continue
            if (
                current.st_dev != status.st_dev
                or current.st_ino != status.st_ino
                or not stat.S_ISDIR(current.st_mode)
                or stat.S_ISLNK(current.st_mode)
            ):
                raise ProvisionError("Zig active-cache entry changed during claim")
            destination = trash_root / candidate.name
            try:
                candidate.rename(destination)
            except FileNotFoundError:
                continue
            moved.append(destination)
        finally:
            os.close(descriptor)
    return moved


def move_owned_active_to_trash(run_directory: Path, trash_root: Path) -> Path:
    """Atomically detach the caller's leased extraction while coordinated."""

    destination = trash_root / run_directory.name
    run_directory.rename(destination)
    return destination


def create_active_lease(active_root: Path, spec: ToolchainSpec) -> tuple[Path, int]:
    run_directory = Path(
        tempfile.mkdtemp(prefix=f"zig-{ZIG_VERSION}-{spec.target}-", dir=active_root)
    )
    descriptor: int | None = None
    try:
        descriptor = open_private_file(run_directory / LEASE_NAME, exclusive=True)
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return run_directory, descriptor
    except BaseException:
        if descriptor is not None:
            os.close(descriptor)
        shutil.rmtree(run_directory, ignore_errors=True)
        raise


@contextmanager
def provisioned_zig(
    cache_root: Path,
    spec: ToolchainSpec,
    run: CommandRunner = subprocess.run,
) -> Iterator[Path]:
    """Yield one leased fresh extraction and recover stale owned entries."""

    cache_root = cache_root.resolve()
    cache_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    run_directory: Path | None = None
    lease_descriptor: int | None = None
    pending_trashes: list[PrivateTrash] = []
    try:
        with cache_lock(cache_root):
            archive_entries = prune_partial_archives(cache_root, spec)
            archive = ensure_archive(
                cache_root,
                spec,
                run,
                known_archive_entries=archive_entries,
            )
        active_root = cache_root / "active"
        active_root.mkdir(mode=0o700, exist_ok=True)
        for attempt in range(RECOVERY_ADMISSION_ATTEMPTS):
            claimed_existing = False
            with cache_lock(cache_root, ACTIVE_LOCK_NAME):
                active_names = sorted(
                    bounded_directory_names(
                        active_root,
                        ACTIVE_DIRECTORY_SCAN_CAP,
                        "Zig active cache",
                    )
                )
                cache_names = sorted(
                    bounded_directory_names(
                        cache_root,
                        CACHE_DIRECTORY_SCAN_CAP,
                        "Zig cache root",
                    )
                )
                trash_names = [
                    name for name in cache_names if name.startswith(TRASH_PREFIX)
                ]
                missing_cursors = sum(
                    1
                    for cursor_name, candidates in (
                        (TRASH_CURSOR_NAME, trash_names),
                        (ACTIVE_CURSOR_NAME, active_names),
                    )
                    if candidates and cursor_name not in cache_names
                )
                can_create_missing_cursors = (
                    len(cache_names) + missing_cursors + 2
                    <= CACHE_DIRECTORY_CAPACITY
                )
                if TRASH_CURSOR_NAME in cache_names or can_create_missing_cursors:
                    trash_window = rotating_trash_window(cache_root, trash_names)
                else:
                    if len(trash_names) > TRASH_SCAN_LIMIT:
                        raise ProvisionError(
                            "Zig cache root has no capacity for fair trash recovery"
                        )
                    trash_window = trash_names[:TRASH_SCAN_LIMIT]
                if ACTIVE_CURSOR_NAME in cache_names or can_create_missing_cursors:
                    active_window = rotating_active_window(cache_root, active_names)
                else:
                    if len(active_names) > STALE_ACTIVE_SCAN_LIMIT:
                        raise ProvisionError(
                            "Zig cache root has no capacity for fair active recovery"
                        )
                    active_window = active_names[:STALE_ACTIVE_SCAN_LIMIT]
                with defer_termination_signals():
                    retire_orphan_trash_leases(cache_root, trash_window)
                    claimed = claim_abandoned_trash(cache_root, trash_window)
                    pending_trashes.extend(claimed)
                    claimed_existing = bool(claimed)
                if not claimed_existing:
                    current_root_count = len(
                        bounded_directory_names(
                            cache_root,
                            CACHE_DIRECTORY_SCAN_CAP,
                            "Zig cache root",
                        )
                    )
                    if current_root_count + 2 > CACHE_DIRECTORY_CAPACITY:
                        raise ProvisionError(
                            "Zig cache root has no capacity for trash ownership"
                        )
                    with defer_termination_signals():
                        stale_trash = create_private_trash(cache_root)
                        pending_trashes.append(stale_trash)
                        moved = move_stale_active_to_trash(
                            active_root,
                            stale_trash.path,
                            candidate_names=active_window,
                        )
                        prospective_active = len(active_names) - len(moved) + 1
                        if prospective_active > ACTIVE_DIRECTORY_CAPACITY:
                            raise ProvisionError(
                                "Zig active cache has no capacity for another run"
                            )
                        root_with_trash = current_root_count + 2
                        if (
                            root_with_trash + 2 * prospective_active
                            > CACHE_DIRECTORY_CAPACITY
                        ):
                            raise ProvisionError(
                                "Zig cache root cannot reserve cleanup capacity"
                            )
                        run_directory, lease_descriptor = create_active_lease(
                            active_root, spec
                        )
            if not claimed_existing:
                break
            while pending_trashes:
                remove_private_trash(pending_trashes.pop())
            if attempt + 1 == RECOVERY_ADMISSION_ATTEMPTS:
                raise ProvisionError(
                    "bounded Zig trash recovery requires another retry"
                )
        while pending_trashes:
            remove_private_trash(pending_trashes.pop())
        install_dir = run_directory / "toolchain"
        extract_archive(archive, install_dir, run)
        executable = install_dir / "zig"
        if executable_version(executable, run) != ZIG_VERSION:
            raise ProvisionError(f"extracted executable is not Zig {ZIG_VERSION}")
        write_marker(install_dir, spec)
        yield executable.resolve(strict=True)
    finally:
        primary_failure = sys.exc_info()[1]
        cleanup_failure: BaseException | None = None
        owned_trash: PrivateTrash | None = None
        try:
            if run_directory is not None:
                try:
                    with cache_lock(cache_root, ACTIVE_LOCK_NAME):
                        cleanup_root_count = len(
                            bounded_directory_names(
                                cache_root,
                                CACHE_DIRECTORY_SCAN_CAP,
                                "Zig cache root",
                            )
                        )
                        if cleanup_root_count + 2 > CACHE_DIRECTORY_CAPACITY:
                            raise ProvisionError(
                                "Zig cache root exhausted reserved cleanup capacity"
                            )
                        with defer_termination_signals():
                            owned_trash = create_private_trash(cache_root)
                            move_owned_active_to_trash(
                                run_directory, owned_trash.path
                            )
                    trash_to_remove = owned_trash
                    owned_trash = None
                    remove_private_trash(trash_to_remove)
                except BaseException as error:
                    cleanup_failure = error
        finally:
            if owned_trash is not None:
                try:
                    remove_private_trash(owned_trash)
                except BaseException as error:
                    if cleanup_failure is None:
                        cleanup_failure = error
            while pending_trashes:
                trash = pending_trashes.pop()
                try:
                    remove_private_trash(trash)
                except BaseException as error:
                    if cleanup_failure is None:
                        cleanup_failure = error
            if lease_descriptor is not None:
                try:
                    os.close(lease_descriptor)
                except BaseException as error:
                    if cleanup_failure is None:
                        cleanup_failure = error

        if primary_failure is None and cleanup_failure is not None:
            if isinstance(cleanup_failure, (OSError, ProvisionError)):
                raise ProvisionError(
                    "could not remove the temporary Zig toolchain"
                ) from cleanup_failure
            raise cleanup_failure
