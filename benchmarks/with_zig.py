#!/usr/bin/env python3
"""Run the pinned upstream harness with an ephemeral exact Zig toolchain."""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
from typing import Sequence


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
sys.dont_write_bytecode = True
os.environ["PYTHONDONTWRITEBYTECODE"] = "1"

from scripts.provision_zig import (  # noqa: E402
    ProvisionError,
    host_spec,
    provisioned_zig,
)


VALIDATED_UPSTREAM_OPTIONS = (
    "--output",
    "--runner-class",
    "--scratch-dir",
    "--upstream-dir",
)
FORWARDED_SIGNALS = tuple(
    candidate
    for candidate in (
        getattr(signal, "SIGHUP", None),
        signal.SIGINT,
        signal.SIGTERM,
    )
    if candidate is not None
)
SIGNAL_GRACE_SECONDS = 5.0
KILL_GRACE_SECONDS = 5.0


class CaughtSignal(BaseException):
    def __init__(self, signum: int) -> None:
        self.signum = signum


class ChildSupervisor:
    def __init__(self) -> None:
        self.child: subprocess.Popen | None = None
        self.caught_signal: int | None = None
        self.signal_forwarded = False
        self.spawning = False

    def forward(self, signum: int) -> None:
        child = self.child
        if child is None or child.poll() is not None:
            return
        try:
            os.killpg(child.pid, signum)
        except ProcessLookupError:
            pass

    def handle_signal(self, signum: int, _frame: object) -> None:
        if self.caught_signal is not None:
            return
        self.caught_signal = signum
        if self.child is not None:
            self.forward_once(signum)
        if not self.spawning:
            raise CaughtSignal(signum)

    @contextmanager
    def signal_handlers(self):
        previous = {signum: signal.getsignal(signum) for signum in FORWARDED_SIGNALS}
        try:
            for signum in FORWARDED_SIGNALS:
                signal.signal(signum, self.handle_signal)
            yield
        finally:
            for signum, handler in previous.items():
                signal.signal(signum, handler)

    def forward_once(self, signum: int) -> None:
        if not self.signal_forwarded:
            self.forward(signum)
            self.signal_forwarded = True

    def reap_after_signal(self, child: subprocess.Popen) -> None:
        try:
            child.communicate(timeout=SIGNAL_GRACE_SECONDS)
            return
        except subprocess.TimeoutExpired:
            pass
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            child.communicate(timeout=KILL_GRACE_SECONDS)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=KILL_GRACE_SECONDS)

    def run(
        self,
        command: Sequence[str],
        *,
        check: bool = False,
        timeout: float | None = None,
        stdout: object = None,
        stderr: object = None,
        text: bool = False,
    ) -> subprocess.CompletedProcess:
        if self.child is not None:
            raise RuntimeError("the Zig wrapper attempted overlapping child processes")
        if self.caught_signal is not None:
            raise CaughtSignal(self.caught_signal)
        child: subprocess.Popen | None = None
        try:
            self.spawning = True
            try:
                child = subprocess.Popen(
                    command,
                    stdout=stdout,
                    stderr=stderr,
                    text=text,
                    start_new_session=True,
                )
                self.child = child
            finally:
                self.spawning = False
            if self.caught_signal is not None:
                self.forward_once(self.caught_signal)
                raise CaughtSignal(self.caught_signal)
            try:
                captured_stdout, captured_stderr = child.communicate(timeout=timeout)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.communicate(timeout=KILL_GRACE_SECONDS)
                raise
        except CaughtSignal as caught:
            if child is not None:
                self.forward_once(caught.signum)
                self.reap_after_signal(child)
            raise
        finally:
            self.child = None
        completed = subprocess.CompletedProcess(
            command,
            child.returncode,
            captured_stdout,
            captured_stderr,
        )
        if check:
            completed.check_returncode()
        return completed


def default_cache_root() -> Path:
    return Path(tempfile.gettempdir()) / f"machine-god-zig-{os.getuid()}"


def parse_arguments(arguments: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--cache-root",
        type=Path,
        default=default_cache_root(),
        help="checksum-verified archive cache (default: a private OS temporary path)",
    )
    parser.add_argument(
        "--validate-evidence",
        type=Path,
        help="validate the collected evidence before removing the exact Zig toolchain",
    )
    parser.add_argument("--expected-git-sha")
    parser.add_argument("--expected-runner-class")
    parser.add_argument("--fx-binary", type=Path)
    parser.add_argument("--machine-god-binary", type=Path)
    parser.add_argument(
        "upstream_arguments",
        nargs=argparse.REMAINDER,
        help="arguments forwarded to benchmarks/upstream.py after --",
    )
    options = parser.parse_args(arguments)
    if options.upstream_arguments[:1] == ["--"]:
        options.upstream_arguments = options.upstream_arguments[1:]
    validation_values = (
        options.validate_evidence,
        options.expected_git_sha,
        options.expected_runner_class,
        options.fx_binary,
        options.machine_god_binary,
    )
    if any(value is not None for value in validation_values) and not all(
        value is not None for value in validation_values
    ):
        parser.error("evidence validation requires all five validation options")
    if any(
        argument.split("=", 1)[0] in {"--z", "--zi", "--zig"}
        for argument in options.upstream_arguments
    ):
        parser.error("the wrapper exclusively owns the forwarded --zig option")
    if options.validate_evidence is not None:
        bind_validation_to_collection(parser, options)
    return options


def forwarded_option(
    parser: argparse.ArgumentParser, arguments: Sequence[str], name: str
) -> str:
    values: list[str] = []
    index = 0
    while index < len(arguments):
        argument = arguments[index]
        if argument == name:
            if index + 1 == len(arguments):
                parser.error(f"forwarded {name} requires a value")
            values.append(arguments[index + 1])
            index += 2
            continue
        prefix = f"{name}="
        if argument.startswith(prefix):
            values.append(argument[len(prefix) :])
        index += 1
    if len(values) != 1 or not values[0]:
        parser.error(f"evidence validation requires exactly one forwarded {name}")
    return values[0]


def canonical_output_path(path: Path) -> Path:
    requested = path.absolute()
    return requested.parent.resolve() / requested.name


def bind_validation_to_collection(
    parser: argparse.ArgumentParser, options: argparse.Namespace
) -> None:
    forwarded = {
        name: forwarded_option(parser, options.upstream_arguments, name)
        for name in VALIDATED_UPSTREAM_OPTIONS
    }
    output = canonical_output_path(Path(forwarded["--output"]))
    upstream = Path(forwarded["--upstream-dir"]).resolve()
    scratch = Path(forwarded["--scratch-dir"]).resolve()
    if canonical_output_path(options.validate_evidence) != output:
        parser.error("validation evidence must be the forwarded collection output")
    if options.expected_runner_class != forwarded["--runner-class"]:
        parser.error("validation runner class must match the forwarded collection runner")
    if options.fx_binary.resolve() != upstream / "zig-out/bin/fx":
        parser.error("validation fx binary must belong to the forwarded upstream directory")
    if (
        options.machine_god_binary.resolve()
        != scratch / "machine-target/release/machine-god"
    ):
        parser.error(
            "validation machine-god binary must belong to the forwarded scratch directory"
        )
    options.validate_evidence = output
    options.fx_binary = upstream / "zig-out/bin/fx"
    options.machine_god_binary = scratch / "machine-target/release/machine-god"


def upstream_command(zig: Path, arguments: Sequence[str]) -> list[str]:
    return [
        sys.executable,
        str(ROOT / "benchmarks/upstream.py"),
        *arguments,
        "--zig",
        str(zig),
    ]


def validation_command(options: argparse.Namespace) -> list[str]:
    return [
        sys.executable,
        str(ROOT / "benchmarks/check.py"),
        str(options.validate_evidence),
        "--expected-git-sha",
        options.expected_git_sha,
        "--expected-runner-class",
        options.expected_runner_class,
        "--fx-binary",
        str(options.fx_binary.resolve(strict=True)),
        "--machine-god-binary",
        str(options.machine_god_binary.resolve(strict=True)),
    ]


def run_benchmark(options: argparse.Namespace, supervisor: ChildSupervisor) -> int:
    with provisioned_zig(
        options.cache_root, host_spec(), run=supervisor.run
    ) as zig:
        completed = supervisor.run(
            upstream_command(zig, options.upstream_arguments), check=False
        )
        if completed.returncode == 0 and options.validate_evidence is not None:
            completed = supervisor.run(validation_command(options), check=False)
    return completed.returncode if 0 <= completed.returncode <= 125 else 1


def main(arguments: Sequence[str] | None = None) -> int:
    options = parse_arguments(sys.argv[1:] if arguments is None else arguments)
    supervisor = ChildSupervisor()
    try:
        with supervisor.signal_handlers():
            return run_benchmark(options, supervisor)
    except CaughtSignal as caught:
        return 128 + caught.signum
    except (OSError, ProvisionError) as error:
        print(f"could not run pinned upstream benchmark: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
