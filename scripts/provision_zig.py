#!/usr/bin/env python3
"""Provision the exact Zig toolchain used to build the pinned fx benchmark.

The Rust product does not use Zig. This helper installs Zig only beneath an
explicit benchmark-tool directory and prints the resulting executable path.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from typing import Sequence


ZIG_VERSION = "0.16.0"
DOWNLOAD_BASE = f"https://ziglang.org/download/{ZIG_VERSION}"
MARKER_NAME = ".machine-god-zig.json"
DOWNLOAD_TIMEOUT_SECONDS = 300
DOWNLOAD_RETRIES = 3


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


def executable_version(executable: Path) -> str | None:
    try:
        completed = subprocess.run(
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


def download_archive(destination: Path, spec: ToolchainSpec) -> None:
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
        completed = subprocess.run(command, check=False, timeout=930)
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


def extract_archive(archive: Path, destination: Path) -> None:
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
        completed = subprocess.run(command, check=False, timeout=120)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ProvisionError(f"could not extract the Zig archive: {error}") from error
    if completed.returncode != 0:
        raise ProvisionError("could not extract the Zig archive")


def ensure_archive(install_root: Path, spec: ToolchainSpec) -> Path:
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

    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{spec.archive_name}.", dir=archive_root
    )
    os.close(descriptor)
    temporary = Path(temporary_name)
    try:
        download_archive(temporary, spec)
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


def provision(install_root: Path, spec: ToolchainSpec) -> Path:
    install_root = install_root.resolve()
    install_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    archive = ensure_archive(install_root, spec)
    install_parent = install_root / "installs"
    install_parent.mkdir(mode=0o700, exist_ok=True)
    run_dir = Path(
        tempfile.mkdtemp(prefix=f"zig-{ZIG_VERSION}-{spec.target}-", dir=install_parent)
    )
    install_dir = run_dir / "toolchain"
    extract_archive(archive, install_dir)
    executable = install_dir / "zig"
    if executable_version(executable) != ZIG_VERSION:
        raise ProvisionError(f"extracted executable is not Zig {ZIG_VERSION}")
    (install_dir / MARKER_NAME).write_text(
        json.dumps(marker_payload(spec), sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    return executable.resolve(strict=True)


def parse_arguments(arguments: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--install-root",
        type=Path,
        default=Path(tempfile.gettempdir()) / f"machine-god-zig-{os.getuid()}",
        help="private toolchain directory (default: an OS temporary directory)",
    )
    return parser.parse_args(arguments)


def main(arguments: Sequence[str] | None = None) -> int:
    options = parse_arguments(sys.argv[1:] if arguments is None else arguments)
    try:
        executable = provision(options.install_root, host_spec())
    except (OSError, ProvisionError) as error:
        print(f"could not provision Zig: {error}", file=sys.stderr)
        return 1
    print(executable)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
