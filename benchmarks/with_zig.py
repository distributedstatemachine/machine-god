#!/usr/bin/env python3
"""Run the pinned upstream harness with an ephemeral exact Zig toolchain."""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import time
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
ANCHOR_READY_SECONDS = 5.0
FINAL_SIGNAL_ATTEMPTS = 3
REAP_ATTEMPTS = 3


class CaughtSignal(BaseException):
    def __init__(self, signum: int) -> None:
        self.signum = signum


class ChildSupervisor:
    def __init__(self) -> None:
        self.child: subprocess.Popen | None = None
        self.group_leader: subprocess.Popen | None = None
        self.caught_signal: int | None = None
        self.signal_forwarded = False
        self.spawning = False
        self.cleaning = False
        self.anchor_ready = False
        self.command_joined = False
        self.signal_mask_valid = True

    def forward(self, signum: int) -> None:
        group_leader = self.group_leader
        if group_leader is None:
            return
        try:
            os.killpg(group_leader.pid, signum)
        except BaseException:
            pass

    def handle_signal(self, signum: int, _frame: object) -> None:
        if self.caught_signal is not None:
            return
        self.caught_signal = signum
        if self.anchor_ready and self.command_joined:
            self.forward_once(signum)
        if not self.spawning and not self.cleaning:
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

    @staticmethod
    def signal_group(group_leader: subprocess.Popen, signum: int) -> None:
        try:
            os.killpg(group_leader.pid, signum)
        except BaseException:
            pass

    @staticmethod
    def terminate_group(group_leader: subprocess.Popen) -> bool:
        """Boundedly deliver the final group signal while the PGID is reserved."""
        for _attempt in range(FINAL_SIGNAL_ATTEMPTS):
            try:
                os.killpg(group_leader.pid, signal.SIGKILL)
                return True
            except ProcessLookupError:
                return True
            except BaseException:
                pass
        return False

    @staticmethod
    def terminate_process(process: subprocess.Popen) -> None:
        """Best-effort identity-safe fallback for an unreaped direct child."""
        for _attempt in range(FINAL_SIGNAL_ATTEMPTS):
            try:
                process.kill()
                return
            except ProcessLookupError:
                return
            except BaseException:
                pass

    @staticmethod
    def remaining(deadline: float) -> float:
        try:
            return max(0.0, deadline - time.monotonic())
        except BaseException:
            return 0.0

    def wait_until(self, deadline: float) -> None:
        while True:
            remaining = self.remaining(deadline)
            if remaining <= 0.0:
                return
            try:
                time.sleep(min(0.01, remaining))
            except BaseException:
                # A second asynchronous exception must not displace the one
                # whose unwinding is already in progress.
                pass

    def close_group_authority(self, group_leader: subprocess.Popen) -> None:
        if self.group_leader is group_leader:
            self.group_leader = None

    def communicate_until(self, child: subprocess.Popen, deadline: float) -> bool:
        for _attempt in range(REAP_ATTEMPTS):
            remaining = self.remaining(deadline)
            if remaining <= 0.0:
                return False
            try:
                child.communicate(timeout=remaining)
                return True
            except subprocess.TimeoutExpired:
                continue
            except BaseException:
                pass
        return False

    def reap_until(self, process: subprocess.Popen, deadline: float) -> bool:
        """Boundedly retry wait so one interruption cannot abandon a child."""
        for _attempt in range(REAP_ATTEMPTS):
            try:
                process.wait(timeout=self.remaining(deadline))
                return True
            except subprocess.TimeoutExpired:
                pass
            except BaseException:
                pass
        return False

    @staticmethod
    def close_descriptor(descriptor: int) -> bool:
        """Close detached descriptor authority once without reusing its number."""
        try:
            os.close(descriptor)
            return True
        except BaseException:
            # close(2) may have released the descriptor before surfacing an
            # interruption. Retrying its numeric value could close unrelated
            # authority that reused the number, so an ambiguous result fails
            # closed without another close attempt.
            return False

    def restore_signal_mask(self, previous_mask: object) -> bool:
        for _attempt in range(REAP_ATTEMPTS):
            try:
                signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
                return True
            except BaseException:
                pass
        self.signal_mask_valid = False
        return False

    @staticmethod
    def wait_anchor_ready(ready_descriptor: int) -> None:
        try:
            deadline = time.monotonic() + ANCHOR_READY_SECONDS
        except BaseException as error:
            raise RuntimeError(
                "benchmark process-group anchor failed to become ready"
            ) from error
        while True:
            try:
                remaining = max(0.0, deadline - time.monotonic())
                if remaining <= 0.0:
                    raise RuntimeError(
                        "benchmark process-group anchor failed to become ready"
                    )
                readable, _, _ = select.select(
                    [ready_descriptor], [], [], remaining
                )
                if not readable or os.read(ready_descriptor, 1) != b"R":
                    raise RuntimeError(
                        "benchmark process-group anchor failed to become ready"
                    )
                return
            except InterruptedError:
                continue

    def terminate_and_reap(
        self,
        child: subprocess.Popen,
        group_leader: subprocess.Popen,
        *,
        initial_signal: int | None,
        grace_seconds: float | None = None,
    ) -> bool:
        """Best-effort bounded cleanup which never replaces an active exception."""
        previous_cleaning = self.cleaning
        self.cleaning = True
        try:
            if initial_signal is not None:
                self.signal_group(group_leader, initial_signal)

            if grace_seconds is None:
                grace_seconds = SIGNAL_GRACE_SECONDS
            try:
                grace_deadline = time.monotonic() + max(0.0, grace_seconds)
            except BaseException:
                grace_deadline = 0.0
            child_completed = self.communicate_until(child, grace_deadline)
            if not child_completed:
                self.wait_until(grace_deadline)

            # The unreaped anchor reserves the numeric PGID through every final
            # group-signal attempt. If group signaling fails, the still-owned
            # Popen handles provide identity-safe direct-child fallbacks.
            group_terminated = self.terminate_group(group_leader)
            if not group_terminated:
                self.terminate_process(child)
                self.terminate_process(group_leader)

            # Close forwarding authority before either child is reaped, since
            # the numeric PGID can become reusable as soon as the anchor exits.
            # Reaping belongs in finally so an asynchronous exception injected
            # at this boundary cannot abandon either direct child.
            try:
                self.close_group_authority(group_leader)
            finally:
                try:
                    kill_deadline = time.monotonic() + max(
                        0.0, KILL_GRACE_SECONDS
                    )
                except BaseException:
                    kill_deadline = 0.0
                anchor_reaped = self.reap_until(group_leader, kill_deadline)
                child_communicated = self.communicate_until(child, kill_deadline)
                child_reaped = child_communicated or self.reap_until(
                    child, kill_deadline
                )
            return group_terminated and anchor_reaped and child_reaped
        finally:
            self.cleaning = previous_cleaning

    def terminate_anchor(self, group_leader: subprocess.Popen) -> None:
        previous_cleaning = self.cleaning
        self.cleaning = True
        try:
            if not self.terminate_group(group_leader):
                self.terminate_process(group_leader)
            try:
                self.close_group_authority(group_leader)
            finally:
                try:
                    kill_deadline = time.monotonic() + max(
                        0.0, KILL_GRACE_SECONDS
                    )
                except BaseException:
                    kill_deadline = 0.0
                self.reap_until(group_leader, kill_deadline)
        finally:
            self.cleaning = previous_cleaning

    @staticmethod
    def anchor_command(ready_descriptor: int | None = None) -> list[str]:
        ready_statement = ""
        arguments: list[str] = []
        if ready_descriptor is not None:
            ready_statement = (
                "ready=int(sys.argv[1])\n"
                "signal.pthread_sigmask(signal.SIG_UNBLOCK,handled)\n"
                "os.write(ready,b'R')\n"
                "os.close(ready)\n"
            )
            arguments.append(str(ready_descriptor))
        return [
            sys.executable,
            "-c",
            (
                "import os,signal,sys,time\n"
                "handled=[]\n"
                "for name in ('SIGHUP','SIGINT','SIGTERM'):\n"
                " signum=getattr(signal,name,None)\n"
                " if signum is not None:\n"
                "  signal.signal(signum,signal.SIG_IGN)\n"
                "  handled.append(signum)\n"
                f"{ready_statement}"
                "time.sleep(86400)\n"
            ),
            *arguments,
        ]

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
        if self.child is not None or self.group_leader is not None:
            raise RuntimeError("the Zig wrapper attempted overlapping child processes")
        if not self.signal_mask_valid:
            raise RuntimeError("the Zig wrapper signal mask state is invalid")
        if self.caught_signal is not None:
            raise CaughtSignal(self.caught_signal)
        child: subprocess.Popen | None = None
        group_leader: subprocess.Popen | None = None
        ready_descriptor: int | None = None
        ready_write_descriptor: int | None = None
        try:
            self.spawning = True
            try:
                ready_descriptor, ready_write_descriptor = os.pipe()
                previous_mask = signal.pthread_sigmask(
                    signal.SIG_BLOCK, FORWARDED_SIGNALS
                )
                launch_error: BaseException | None = None
                try:
                    group_leader = subprocess.Popen(
                        self.anchor_command(ready_write_descriptor),
                        stdin=subprocess.DEVNULL,
                        stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL,
                        process_group=0,
                        pass_fds=(ready_write_descriptor,),
                    )
                    self.group_leader = group_leader
                except BaseException as error:
                    launch_error = error
                detached_write_descriptor = ready_write_descriptor
                ready_write_descriptor = None
                write_closed = self.close_descriptor(detached_write_descriptor)
                mask_restored = self.restore_signal_mask(previous_mask)
                if launch_error is not None:
                    raise launch_error
                if not mask_restored:
                    raise RuntimeError(
                        "the Zig wrapper signal mask state is invalid"
                    )
                if not write_closed:
                    raise RuntimeError(
                        "benchmark process-group readiness descriptor did not close"
                    )
                self.wait_anchor_ready(ready_descriptor)
                detached_ready_descriptor = ready_descriptor
                ready_descriptor = None
                if not self.close_descriptor(detached_ready_descriptor):
                    raise RuntimeError(
                        "benchmark process-group readiness descriptor did not close"
                    )
                self.anchor_ready = True
                child = subprocess.Popen(
                    command,
                    stdout=stdout,
                    stderr=stderr,
                    text=text,
                    process_group=group_leader.pid,
                )
                self.child = child
                self.command_joined = True
                if self.caught_signal is not None:
                    self.forward_once(self.caught_signal)
            finally:
                self.spawning = False
            if self.caught_signal is not None:
                raise CaughtSignal(self.caught_signal)
            captured_stdout, captured_stderr = child.communicate(timeout=timeout)
            completed = subprocess.CompletedProcess(
                command,
                child.returncode,
                captured_stdout,
                captured_stderr,
            )
            cleanup_complete = self.terminate_and_reap(
                child,
                group_leader,
                initial_signal=None,
                grace_seconds=0.0,
            )
            if self.caught_signal is not None:
                raise CaughtSignal(self.caught_signal)
            if not cleanup_complete:
                raise RuntimeError("benchmark process-group cleanup was incomplete")
            if check:
                completed.check_returncode()
            return completed
        except BaseException as error:
            owns_group = (
                group_leader is not None and self.group_leader is group_leader
            )
            if child is not None and group_leader is not None and owns_group:
                try:
                    if isinstance(error, CaughtSignal):
                        self.forward_once(error.signum)
                        self.terminate_and_reap(
                            child, group_leader, initial_signal=None
                        )
                    elif isinstance(error, subprocess.TimeoutExpired):
                        self.terminate_and_reap(
                            child,
                            group_leader,
                            initial_signal=None,
                            grace_seconds=0.0,
                        )
                    else:
                        self.terminate_and_reap(
                            child, group_leader, initial_signal=signal.SIGTERM
                        )
                except BaseException:
                    # Cleanup is deliberately subordinate to the active
                    # exception, including asynchronous exceptions delivered
                    # while cleanup itself is running.
                    pass
            elif group_leader is not None and owns_group:
                self.terminate_anchor(group_leader)
            raise
        finally:
            if ready_write_descriptor is not None:
                detached_write_descriptor = ready_write_descriptor
                ready_write_descriptor = None
                self.close_descriptor(detached_write_descriptor)
            if ready_descriptor is not None:
                detached_ready_descriptor = ready_descriptor
                ready_descriptor = None
                self.close_descriptor(detached_ready_descriptor)
            self.child = None
            self.group_leader = None
            self.anchor_ready = False
            self.command_joined = False


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
