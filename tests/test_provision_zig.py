from __future__ import annotations

from contextlib import contextmanager, redirect_stderr
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

from benchmarks import with_zig
from scripts import provision_zig


class ProvisionZigTests(unittest.TestCase):
    def test_supported_hosts_are_pinned(self) -> None:
        self.assertEqual(
            provision_zig.host_spec("Darwin", "arm64").target,
            "aarch64-macos",
        )
        self.assertEqual(
            provision_zig.host_spec("Linux", "x86_64").sha256,
            "70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00",
        )

    def test_unknown_host_fails_closed(self) -> None:
        with self.assertRaisesRegex(provision_zig.ProvisionError, "unsupported"):
            provision_zig.host_spec("Plan9", "mips")

    def test_default_install_root_is_outside_the_checkout(self) -> None:
        options = with_zig.parse_arguments([])
        checkout = Path(__file__).resolve().parents[1]
        self.assertFalse(options.cache_root.resolve().is_relative_to(checkout))

    def test_wrapper_forbids_checkout_bytecode_for_itself_and_child(self) -> None:
        self.assertTrue(with_zig.sys.dont_write_bytecode)
        self.assertEqual(with_zig.os.environ["PYTHONDONTWRITEBYTECODE"], "1")

    def test_validated_archive_requires_exact_bytes_and_regular_file(self) -> None:
        payload = b"pinned archive"
        spec = provision_zig.ToolchainSpec(
            "test-host", hashlib.sha256(payload).hexdigest(), len(payload)
        )
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "archive.tar.xz"
            archive.write_bytes(payload)
            self.assertEqual(
                provision_zig.validated_archive(archive, spec), archive.resolve()
            )
            archive.write_bytes(payload + b"tampered")
            self.assertIsNone(provision_zig.validated_archive(archive, spec))

    def test_ensure_archive_rehashes_valid_cache_without_download(self) -> None:
        payload = b"pinned archive"
        spec = provision_zig.ToolchainSpec(
            "test-host", hashlib.sha256(payload).hexdigest(), len(payload)
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archives" / spec.archive_name
            archive.parent.mkdir()
            archive.write_bytes(payload)
            with mock.patch.object(provision_zig, "download_archive") as download:
                self.assertEqual(
                    provision_zig.ensure_archive(root, spec), archive.resolve()
                )
            download.assert_not_called()

    def test_ensure_archive_refuses_invalid_cache(self) -> None:
        payload = b"pinned archive"
        spec = provision_zig.ToolchainSpec(
            "test-host", hashlib.sha256(payload).hexdigest(), len(payload)
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archives" / spec.archive_name
            archive.parent.mkdir()
            archive.write_bytes(b"tampered")
            with self.assertRaisesRegex(provision_zig.ProvisionError, "move it aside"):
                provision_zig.ensure_archive(root, spec)

    def test_provisioned_context_cleans_each_successful_install(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archive.tar.xz"
            archive.write_bytes(b"fixture")

            def fake_extract(
                _archive: Path, destination: Path, _run: object = None
            ) -> None:
                destination.mkdir()
                executable = destination / "zig"
                executable.write_text("#!/bin/sh\nprintf '0.16.0\\n'\n", encoding="utf-8")
                executable.chmod(executable.stat().st_mode | stat.S_IXUSR)

            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                mock.patch.object(
                    provision_zig, "extract_archive", side_effect=fake_extract
                ),
            ):
                with provision_zig.provisioned_zig(root, spec) as first:
                    first_parent = first.parents[1]
                    self.assertTrue(first.is_file())
                self.assertFalse(first_parent.exists())
                with provision_zig.provisioned_zig(root, spec) as second:
                    second_parent = second.parents[1]
                    self.assertTrue(second.is_file())
                self.assertFalse(second_parent.exists())
            self.assertEqual(list((root / "active").iterdir()), [])

    def test_provisioned_context_cleans_wrong_version_failure(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archive.tar.xz"
            archive.write_bytes(b"fixture")

            def fake_extract(
                _archive: Path, destination: Path, _run: object = None
            ) -> None:
                destination.mkdir()
                executable = destination / "zig"
                executable.write_text("#!/bin/sh\nprintf '0.14.1\\n'\n", encoding="utf-8")
                executable.chmod(executable.stat().st_mode | stat.S_IXUSR)

            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                mock.patch.object(provision_zig, "extract_archive", side_effect=fake_extract),
            ):
                with self.assertRaisesRegex(provision_zig.ProvisionError, "not Zig 0.16.0"):
                    with provision_zig.provisioned_zig(root, spec):
                        self.fail("wrong Zig version was yielded")
            self.assertEqual(list((root / "active").iterdir()), [])

    def test_provisioned_context_cleans_extraction_and_marker_failures(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archive.tar.xz"
            archive.write_bytes(b"fixture")

            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                mock.patch.object(
                    provision_zig,
                    "extract_archive",
                    side_effect=provision_zig.ProvisionError("extract failed"),
                ),
                self.assertRaisesRegex(provision_zig.ProvisionError, "extract failed"),
            ):
                with provision_zig.provisioned_zig(root, spec):
                    self.fail("failed extraction was yielded")
            self.assertEqual(list((root / "active").iterdir()), [])

            def fake_extract(
                _archive: Path, destination: Path, _run: object = None
            ) -> None:
                destination.mkdir()
                executable = destination / "zig"
                executable.write_text(
                    "#!/bin/sh\nprintf '0.16.0\\n'\n", encoding="utf-8"
                )
                executable.chmod(executable.stat().st_mode | stat.S_IXUSR)

            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                mock.patch.object(
                    provision_zig, "extract_archive", side_effect=fake_extract
                ),
                mock.patch.object(
                    provision_zig,
                    "write_marker",
                    side_effect=OSError("marker failed"),
                ),
                self.assertRaisesRegex(OSError, "marker failed"),
            ):
                with provision_zig.provisioned_zig(root, spec):
                    self.fail("failed marker write was yielded")
            self.assertEqual(list((root / "active").iterdir()), [])

    def test_provisioner_prunes_stale_but_preserves_live_leased_runs(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            active = root / "active"
            active.mkdir()
            stale = active / "zig-stale"
            stale.mkdir()
            (stale / provision_zig.LEASE_NAME).write_text("stale", encoding="utf-8")
            live = active / "zig-live"
            live.mkdir()
            live_lease = provision_zig.open_private_file(
                live / provision_zig.LEASE_NAME, exclusive=True
            )
            fcntl.flock(live_lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
            archive_root = root / "archives"
            archive_root.mkdir()
            partial = archive_root / f".{spec.archive_name}.interrupted"
            partial.write_bytes(b"partial")
            archive = root / "archive.tar.xz"
            archive.write_bytes(b"fixture")

            def fake_extract(
                _archive: Path, destination: Path, _run: object = None
            ) -> None:
                destination.mkdir()
                executable = destination / "zig"
                executable.write_text(
                    "#!/bin/sh\nprintf '0.16.0\\n'\n", encoding="utf-8"
                )
                executable.chmod(executable.stat().st_mode | stat.S_IXUSR)

            try:
                with (
                    mock.patch.object(
                        provision_zig, "ensure_archive", return_value=archive
                    ),
                    mock.patch.object(
                        provision_zig, "extract_archive", side_effect=fake_extract
                    ),
                ):
                    with provision_zig.provisioned_zig(root, spec):
                        self.assertFalse(stale.exists())
                        self.assertFalse(partial.exists())
                        self.assertTrue(live.exists())
                    self.assertTrue(live.exists())
                    os.close(live_lease)
                    live_lease = -1
                    with provision_zig.provisioned_zig(root, spec):
                        self.assertFalse(live.exists())
            finally:
                if live_lease >= 0:
                    os.close(live_lease)
            self.assertEqual(list(active.iterdir()), [])

    def test_post_acquisition_signal_cleans_the_owned_lease(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        supervisor = with_zig.ChildSupervisor()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archive.tar.xz"
            archive.write_bytes(b"fixture")
            real_create = provision_zig.create_active_lease

            def create_then_interrupt(
                active_root: Path, selected: provision_zig.ToolchainSpec
            ) -> tuple[Path, int]:
                lease = real_create(active_root, selected)
                os.kill(os.getpid(), signal.SIGTERM)
                return lease

            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                mock.patch.object(
                    provision_zig,
                    "create_active_lease",
                    side_effect=create_then_interrupt,
                ),
                supervisor.signal_handlers(),
                self.assertRaises(with_zig.CaughtSignal) as caught,
            ):
                with provision_zig.provisioned_zig(root, spec, run=supervisor.run):
                    self.fail("interrupted acquisition yielded a Zig executable")
            self.assertEqual(caught.exception.signum, signal.SIGTERM)
            self.assertEqual(list((root / "active").iterdir()), [])

    def test_partial_lease_acquisition_closes_its_descriptor(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            active = Path(temporary) / "active"
            active.mkdir()
            opened: list[int] = []
            real_open = provision_zig.open_private_file

            def capture_open(path: Path, *, exclusive: bool = False) -> int:
                descriptor = real_open(path, exclusive=exclusive)
                opened.append(descriptor)
                return descriptor

            with (
                mock.patch.object(
                    provision_zig,
                    "open_private_file",
                    side_effect=capture_open,
                ),
                mock.patch.object(
                    provision_zig.fcntl,
                    "flock",
                    side_effect=OSError("lock failed"),
                ),
                mock.patch.object(
                    provision_zig.os,
                    "close",
                    wraps=provision_zig.os.close,
                ) as close,
                self.assertRaisesRegex(OSError, "lock failed"),
            ):
                provision_zig.create_active_lease(active, spec)
            self.assertEqual(len(opened), 1)
            self.assertIn(mock.call(opened[0]), close.call_args_list)
            self.assertEqual(list(active.iterdir()), [])

    def test_private_file_closes_its_descriptor_when_fchmod_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "private"
            opened: list[int] = []
            real_open = provision_zig.os.open

            def capture_open(*args, **kwargs) -> int:
                descriptor = real_open(*args, **kwargs)
                opened.append(descriptor)
                return descriptor

            with (
                mock.patch.object(
                    provision_zig.os,
                    "open",
                    side_effect=capture_open,
                ),
                mock.patch.object(
                    provision_zig.os,
                    "fchmod",
                    side_effect=OSError("chmod failed"),
                ),
                self.assertRaisesRegex(OSError, "chmod failed"),
            ):
                provision_zig.open_private_file(path)

            self.assertEqual(len(opened), 1)
            with self.assertRaises(OSError):
                os.fstat(opened[0])

    def test_stale_cleanup_moves_only_one_bounded_batch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            active = root / "active"
            active.mkdir()
            count = provision_zig.STALE_ACTIVE_BATCH_SIZE + 3
            for index in range(count):
                candidate = active / f"zig-stale-{index:03d}"
                candidate.mkdir()
                (candidate / provision_zig.LEASE_NAME).write_text(
                    "stale", encoding="utf-8"
                )
            trash = provision_zig.create_private_trash(root)

            with provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME):
                moved = provision_zig.move_stale_active_to_trash(active, trash.path)

            self.assertEqual(len(moved), provision_zig.STALE_ACTIVE_BATCH_SIZE)
            self.assertEqual(
                len(list(active.iterdir())),
                count - provision_zig.STALE_ACTIVE_BATCH_SIZE,
            )
            self.assertEqual(set(moved), set(trash.path.iterdir()))
            provision_zig.remove_private_trash(trash)

    def test_rotating_active_window_cannot_starve_later_entries(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            names = [
                f"zig-active-{index:03d}"
                for index in range(provision_zig.STALE_ACTIVE_SCAN_LIMIT + 3)
            ]

            with provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME):
                first = provision_zig.rotating_active_window(root, names)
            with provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME):
                second = provision_zig.rotating_active_window(root, names)

            self.assertEqual(len(first), provision_zig.STALE_ACTIVE_SCAN_LIMIT)
            self.assertIn(names[-1], second)
            self.assertNotEqual(first, second)

    def test_abandoned_trash_is_reclaimed_without_racing_live_owner(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            trash = provision_zig.create_private_trash(root)
            (trash.path / "payload").write_text("stale", encoding="utf-8")
            names = [trash.path.name]

            with provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME):
                self.assertEqual(
                    provision_zig.claim_abandoned_trash(root, names), []
                )

            os.close(trash.descriptor)
            with provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME):
                claimed = provision_zig.claim_abandoned_trash(root, names)

            self.assertEqual(len(claimed), 1)
            provision_zig.remove_private_trash(claimed[0])
            self.assertFalse(trash.path.exists())
            self.assertFalse(trash.lease_path.exists())

    def test_stale_recursive_deletion_runs_after_active_lock_release(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            active = root / "active"
            active.mkdir()
            stale = active / "zig-stale"
            stale.mkdir()
            (stale / provision_zig.LEASE_NAME).write_text("stale", encoding="utf-8")
            archive = root / "archive.tar.xz"
            archive.write_bytes(b"fixture")
            observed_unlocked_delete = False
            real_rmtree = provision_zig.shutil.rmtree

            def fake_extract(
                _archive: Path, destination: Path, _run: object = None
            ) -> None:
                destination.mkdir()
                executable = destination / "zig"
                executable.write_text(
                    "#!/bin/sh\nprintf '0.16.0\\n'\n", encoding="utf-8"
                )
                executable.chmod(executable.stat().st_mode | stat.S_IXUSR)

            def verify_stale_delete_is_unlocked(path: Path, *args, **kwargs) -> None:
                nonlocal observed_unlocked_delete
                selected = Path(path)
                if selected.exists() and (selected / "zig-stale").exists():
                    with provision_zig.cache_lock(
                        root, provision_zig.ACTIVE_LOCK_NAME
                    ):
                        observed_unlocked_delete = True
                real_rmtree(path, *args, **kwargs)

            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                mock.patch.object(
                    provision_zig, "extract_archive", side_effect=fake_extract
                ),
                mock.patch.object(
                    provision_zig.shutil,
                    "rmtree",
                    side_effect=verify_stale_delete_is_unlocked,
                ),
            ):
                with provision_zig.provisioned_zig(root, spec):
                    pass

            self.assertTrue(observed_unlocked_delete)

    def test_owned_cleanup_does_not_serialize_recursive_deletion(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archive.tar.xz"
            archive.write_bytes(b"fixture")
            first_entered = threading.Event()
            release_first = threading.Event()
            cleanup_started = threading.Event()
            release_cleanup = threading.Event()
            second_finished = threading.Event()
            errors: list[BaseException] = []
            blocked_cleanup = False
            real_rmtree = provision_zig.shutil.rmtree

            def fake_extract(
                _archive: Path, destination: Path, _run: object = None
            ) -> None:
                destination.mkdir()
                executable = destination / "zig"
                executable.write_text(
                    "#!/bin/sh\nprintf '0.16.0\\n'\n", encoding="utf-8"
                )
                executable.chmod(executable.stat().st_mode | stat.S_IXUSR)

            def pause_first_active_cleanup(path: Path, *args, **kwargs) -> None:
                nonlocal blocked_cleanup
                selected = Path(path)
                is_owned_trash = (
                    selected.name.startswith(provision_zig.TRASH_PREFIX)
                    and selected.exists()
                    and any(
                        child.name.startswith("zig-") for child in selected.iterdir()
                    )
                )
                if is_owned_trash and not blocked_cleanup:
                    blocked_cleanup = True
                    cleanup_started.set()
                    if not release_cleanup.wait(timeout=5.0):
                        raise AssertionError("cleanup synchronization timed out")
                real_rmtree(path, *args, **kwargs)

            def first_run() -> None:
                try:
                    with provision_zig.provisioned_zig(root, spec):
                        first_entered.set()
                        if not release_first.wait(timeout=5.0):
                            raise AssertionError("first run synchronization timed out")
                except BaseException as error:
                    errors.append(error)

            def second_run() -> None:
                try:
                    with provision_zig.provisioned_zig(root, spec):
                        pass
                except BaseException as error:
                    errors.append(error)
                finally:
                    second_finished.set()

            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                mock.patch.object(
                    provision_zig, "extract_archive", side_effect=fake_extract
                ),
                mock.patch.object(
                    provision_zig.shutil,
                    "rmtree",
                    side_effect=pause_first_active_cleanup,
                ),
            ):
                first = threading.Thread(target=first_run)
                first.start()
                self.assertTrue(first_entered.wait(timeout=5.0))
                release_first.set()
                self.assertTrue(cleanup_started.wait(timeout=5.0))
                second = threading.Thread(target=second_run)
                second.start()
                self.assertTrue(second_finished.wait(timeout=2.0))
                release_cleanup.set()
                first.join(timeout=5.0)
                second.join(timeout=5.0)
                self.assertFalse(first.is_alive())
                self.assertFalse(second.is_alive())
            self.assertEqual(errors, [])
            self.assertEqual(list((root / "active").iterdir()), [])

    def test_wrapper_sigterm_reaps_child_and_unwinds_zig_context(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            owned = root / "active/owned"
            signal_record = root / "signal.json"

            @contextmanager
            def fake_provisioned_zig(
                _cache: Path, _spec: object, *, run: object
            ):
                self.assertIsNotNone(run)
                owned.mkdir(parents=True)
                try:
                    yield root / "zig"
                finally:
                    shutil.rmtree(owned)

            child = [
                sys.executable,
                "-c",
                (
                    "import json,os,signal,sys,time\n"
                    "record=sys.argv[1]\n"
                    "blocked=sorted(int(item) for item in "
                    "signal.pthread_sigmask(signal.SIG_BLOCK, []))\n"
                    "received=0\n"
                    "def handle(signum, _frame):\n"
                    " global received\n"
                    " received += 1\n"
                    " if received != 1:\n"
                    "  return\n"
                    " deadline=time.monotonic()+0.2\n"
                    " while time.monotonic() < deadline:\n"
                    "  time.sleep(0.01)\n"
                    " with open(record, 'w', encoding='utf-8') as target:\n"
                    "  json.dump({'blocked':blocked,'received':received},target)\n"
                    " raise SystemExit(0)\n"
                    "signal.signal(signal.SIGTERM,handle)\n"
                    "os.kill(os.getppid(),signal.SIGTERM)\n"
                    "time.sleep(60)\n"
                ),
                str(signal_record),
            ]
            started = time.monotonic()
            with (
                mock.patch.object(with_zig, "host_spec", return_value=object()),
                mock.patch.object(
                    with_zig,
                    "provisioned_zig",
                    side_effect=fake_provisioned_zig,
                ),
                mock.patch.object(with_zig, "upstream_command", return_value=child),
                mock.patch.object(with_zig, "SIGNAL_GRACE_SECONDS", 1.0),
                mock.patch.object(with_zig, "KILL_GRACE_SECONDS", 1.0),
            ):
                exit_code = with_zig.main(
                    ["--cache-root", str(root / "cache"), "--", "--runs", "30"]
                )
            self.assertEqual(exit_code, 128 + signal.SIGTERM)
            self.assertFalse(owned.exists())
            self.assertLess(time.monotonic() - started, 3.0)
            observed = json.loads(signal_record.read_text(encoding="utf-8"))
            self.assertEqual(observed["received"], 1)
            for forwarded in with_zig.FORWARDED_SIGNALS:
                self.assertNotIn(int(forwarded), observed["blocked"])

    def test_wrapper_preserves_arbitrary_launch_exception_during_cleanup(self) -> None:
        failure = OSError("sentinel communicate failure")

        class BrokenChild:
            pid = 424_242
            returncode = None

            def __init__(self) -> None:
                self.communicate_calls = 0

            def communicate(self, *, timeout=None):
                del timeout
                self.communicate_calls += 1
                if self.communicate_calls == 1:
                    raise failure
                raise RuntimeError("cleanup communicate failed")

            def kill(self) -> None:
                raise RuntimeError("cleanup kill failed")

            def wait(self, *, timeout=None) -> None:
                del timeout
                raise RuntimeError("cleanup wait failed")

        child = BrokenChild()
        supervisor = with_zig.ChildSupervisor()
        with (
            mock.patch.object(with_zig.subprocess, "Popen", return_value=child),
            mock.patch.object(with_zig.os, "killpg", return_value=None) as killpg,
            mock.patch.object(with_zig, "SIGNAL_GRACE_SECONDS", 0.0),
            mock.patch.object(with_zig, "KILL_GRACE_SECONDS", 0.0),
            self.assertRaises(OSError) as caught,
        ):
            supervisor.run(["ignored"])

        self.assertIs(caught.exception, failure)
        self.assertIsNone(supervisor.child)
        self.assertIn(mock.call(child.pid, signal.SIGTERM), killpg.call_args_list)
        self.assertIn(mock.call(child.pid, signal.SIGKILL), killpg.call_args_list)

    def test_wrapper_kills_survivor_after_group_leader_exits(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            worker_pid_path = Path(temporary) / "worker.pid"
            worker_source = "\n".join(
                (
                    "import os,signal,sys,time",
                    "signal.signal(signal.SIGTERM, signal.SIG_IGN)",
                    "open(sys.argv[1],'w',encoding='utf-8').write(str(os.getpid()))",
                    "time.sleep(60)",
                )
            )
            leader_source = "\n".join(
                (
                    "import subprocess,sys,time",
                    "subprocess.Popen([sys.executable,'-c',sys.argv[2],sys.argv[1]])",
                    "while not __import__('os').path.exists(sys.argv[1]):",
                    " time.sleep(0.01)",
                    "time.sleep(60)",
                )
            )
            leader = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    leader_source,
                    str(worker_pid_path),
                    worker_source,
                ],
                start_new_session=True,
            )
            worker_pid: int | None = None
            try:
                deadline = time.monotonic() + 5.0
                while time.monotonic() < deadline and not worker_pid_path.exists():
                    time.sleep(0.01)
                self.assertTrue(worker_pid_path.exists())
                worker_pid = int(worker_pid_path.read_text(encoding="utf-8"))

                supervisor = with_zig.ChildSupervisor()
                started = time.monotonic()
                with mock.patch.object(with_zig, "KILL_GRACE_SECONDS", 1.0):
                    supervisor.terminate_and_reap(
                        leader,
                        initial_signal=signal.SIGTERM,
                        grace_seconds=0.1,
                    )
                self.assertLess(time.monotonic() - started, 2.0)
                self.assertIsNotNone(leader.returncode)

                deadline = time.monotonic() + 2.0
                while time.monotonic() < deadline:
                    try:
                        os.kill(worker_pid, 0)
                    except ProcessLookupError:
                        break
                    time.sleep(0.01)
                else:
                    self.fail("SIGTERM-ignoring process-group survivor remained alive")
            finally:
                try:
                    os.killpg(leader.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                try:
                    leader.wait(timeout=1.0)
                except subprocess.TimeoutExpired:
                    leader.kill()
                    leader.wait(timeout=1.0)
                if worker_pid is not None:
                    try:
                        os.kill(worker_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_wrapper_signal_lets_upstream_reap_its_grouped_child(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            owned = root / "active/owned"
            child_pid_path = root / "grouped-child.pid"
            wrapper_pid = os.getpid()
            child_source = (
                "import os,time\n"
                f"open({str(child_pid_path)!r},'w',encoding='utf-8').write(str(os.getpid()))\n"
                f"os.kill({wrapper_pid},15)\n"
                "time.sleep(60)\n"
            )
            helper_source = "\n".join(
                (
                    "import os,sys",
                    f"sys.path.insert(0,{str(with_zig.ROOT / 'benchmarks')!r})",
                    "import upstream",
                    "environment=os.environ.copy()",
                    f"environment[upstream.CONTAINMENT_ENVIRONMENT_KEY]={'e' * 32!r}",
                    "try:",
                    " with upstream.termination_signal_handlers():",
                    "  upstream.run_process(",
                    f"   [sys.executable,'-c',{child_source!r}],",
                    f"   cwd=upstream.Path({str(root)!r}),",
                    "   environment=environment,",
                    "   timeout_seconds=60.0,",
                    "  )",
                    "except upstream.HarnessSignal as caught:",
                    " raise SystemExit(128+caught.signum)",
                )
            )

            @contextmanager
            def fake_provisioned_zig(
                _cache: Path, _spec: object, *, run: object
            ):
                self.assertIsNotNone(run)
                owned.mkdir(parents=True)
                try:
                    yield root / "zig"
                finally:
                    shutil.rmtree(owned)

            with (
                mock.patch.object(with_zig, "host_spec", return_value=object()),
                mock.patch.object(
                    with_zig,
                    "provisioned_zig",
                    side_effect=fake_provisioned_zig,
                ),
                mock.patch.object(
                    with_zig,
                    "upstream_command",
                    return_value=[sys.executable, "-c", helper_source],
                ),
                mock.patch.object(with_zig, "SIGNAL_GRACE_SECONDS", 3.0),
                mock.patch.object(with_zig, "KILL_GRACE_SECONDS", 1.0),
            ):
                exit_code = with_zig.main(["--cache-root", str(root / "cache")])
            self.assertEqual(exit_code, 128 + signal.SIGTERM)
            self.assertFalse(owned.exists())
            child_pid = int(child_pid_path.read_text(encoding="utf-8"))
            deadline = time.monotonic() + 2.0
            while time.monotonic() < deadline:
                try:
                    os.kill(child_pid, 0)
                except ProcessLookupError:
                    break
                time.sleep(0.01)
            else:
                try:
                    os.kill(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.fail("wrapper interruption left the grouped child alive")

    def test_wrapper_binds_ephemeral_zig_and_forwards_arguments(self) -> None:
        zig = Path("/private/toolchain/zig")
        command = with_zig.upstream_command(zig, ["--runs", "30"])
        self.assertEqual(
            command,
            [
                with_zig.sys.executable,
                str(with_zig.ROOT / "benchmarks/upstream.py"),
                "--runs",
                "30",
                "--zig",
                str(zig),
            ],
        )

    def test_wrapper_validation_command_binds_live_exact_binaries(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "evidence.json"
            upstream = root / "upstream"
            scratch = root / "scratch"
            fx = upstream / "zig-out/bin/fx"
            machine = scratch / "machine-target/release/machine-god"
            fx.parent.mkdir(parents=True)
            machine.parent.mkdir(parents=True)
            fx.write_bytes(b"fx")
            machine.write_bytes(b"machine-god")
            options = with_zig.parse_arguments(
                [
                    "--validate-evidence",
                    str(evidence),
                    "--expected-git-sha",
                    "a" * 40,
                    "--expected-runner-class",
                    "runner",
                    "--fx-binary",
                    str(fx),
                    "--machine-god-binary",
                    str(machine),
                    "--",
                    "--runs",
                    "30",
                    "--output",
                    str(evidence),
                    "--runner-class",
                    "runner",
                    "--scratch-dir",
                    str(scratch),
                    "--upstream-dir",
                    str(upstream),
                ]
            )
            self.assertEqual(
                with_zig.validation_command(options),
                [
                    with_zig.sys.executable,
                    str(with_zig.ROOT / "benchmarks/check.py"),
                    str(with_zig.canonical_output_path(evidence)),
                    "--expected-git-sha",
                    "a" * 40,
                    "--expected-runner-class",
                    "runner",
                    "--fx-binary",
                    str(fx.resolve()),
                    "--machine-god-binary",
                    str(machine.resolve()),
                ],
            )

    def test_wrapper_rejects_partial_validation_options(self) -> None:
        diagnostics = io.StringIO()
        with redirect_stderr(diagnostics), self.assertRaises(SystemExit):
            with_zig.parse_arguments(["--expected-git-sha", "a" * 40])
        self.assertIn("requires all five validation options", diagnostics.getvalue())

    def test_wrapper_rejects_forwarded_zig_override_forms(self) -> None:
        for override in (
            ["--zig", "/other/zig"],
            ["--zig=/other/zig"],
            ["--zi", "/other/zig"],
            ["--z=/other/zig"],
        ):
            diagnostics = io.StringIO()
            with (
                self.subTest(override=override),
                redirect_stderr(diagnostics),
                self.assertRaises(SystemExit),
            ):
                with_zig.parse_arguments(["--", *override])
            self.assertIn("exclusively owns", diagnostics.getvalue())

    def test_wrapper_rejects_validation_collection_path_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            diagnostics = io.StringIO()
            with redirect_stderr(diagnostics), self.assertRaises(SystemExit):
                with_zig.parse_arguments(
                    [
                        "--validate-evidence",
                        str(root / "old.json"),
                        "--expected-git-sha",
                        "a" * 40,
                        "--expected-runner-class",
                        "runner",
                        "--fx-binary",
                        str(root / "upstream/zig-out/bin/fx"),
                        "--machine-god-binary",
                        str(root / "scratch/machine-target/release/machine-god"),
                        "--",
                        "--output",
                        str(root / "new.json"),
                        "--runner-class",
                        "runner",
                        "--scratch-dir",
                        str(root / "scratch"),
                        "--upstream-dir",
                        str(root / "upstream"),
                    ]
                )
            self.assertIn("must be the forwarded collection output", diagnostics.getvalue())

    def test_wrapper_binds_every_validation_target_to_the_collection(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            canonical = [
                "--validate-evidence",
                str(root / "evidence.json"),
                "--expected-git-sha",
                "a" * 40,
                "--expected-runner-class",
                "runner",
                "--fx-binary",
                str(root / "upstream/zig-out/bin/fx"),
                "--machine-god-binary",
                str(root / "scratch/machine-target/release/machine-god"),
                "--",
                "--output",
                str(root / "evidence.json"),
                "--runner-class",
                "runner",
                "--scratch-dir",
                str(root / "scratch"),
                "--upstream-dir",
                str(root / "upstream"),
            ]
            mutations = (
                ("runner", 5, "other-runner"),
                ("fx", 7, str(root / "other-fx")),
                ("machine-god", 9, str(root / "other-machine-god")),
                ("duplicate", None, None),
            )
            for label, index, replacement in mutations:
                arguments = canonical.copy()
                if index is None:
                    arguments.extend(["--output", str(root / "evidence.json")])
                else:
                    arguments[index] = replacement
                diagnostics = io.StringIO()
                with (
                    self.subTest(label=label),
                    redirect_stderr(diagnostics),
                    self.assertRaises(SystemExit),
                ):
                    with_zig.parse_arguments(arguments)
                self.assertTrue(diagnostics.getvalue())

    def test_wrapper_keeps_zig_live_through_collection_and_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            upstream = root / "upstream"
            scratch = root / "scratch"
            fx = upstream / "zig-out/bin/fx"
            machine = scratch / "machine-target/release/machine-god"
            fx.parent.mkdir(parents=True)
            machine.parent.mkdir(parents=True)
            fx.write_bytes(b"fx")
            machine.write_bytes(b"machine-god")
            live = False

            @contextmanager
            def fake_provisioned_zig(
                _cache: Path, _spec: object, *, run: object
            ):
                self.assertIsNotNone(run)
                nonlocal live
                live = True
                try:
                    yield root / "zig"
                finally:
                    live = False

            class FakeSupervisor:
                def __init__(self, owner: ProvisionZigTests) -> None:
                    self.owner = owner
                    self.calls = 0

                def run(self, _command: list[str], *, check: bool) -> mock.Mock:
                    self.owner.assertTrue(live)
                    self.owner.assertFalse(check)
                    self.calls += 1
                    return mock.Mock(returncode=0)

            arguments = [
                "--cache-root",
                str(root / "cache"),
                "--validate-evidence",
                str(root / "evidence.json"),
                "--expected-git-sha",
                "a" * 40,
                "--expected-runner-class",
                "runner",
                "--fx-binary",
                str(fx),
                "--machine-god-binary",
                str(machine),
                "--",
                "--runs",
                "30",
                "--output",
                str(root / "evidence.json"),
                "--runner-class",
                "runner",
                "--scratch-dir",
                str(scratch),
                "--upstream-dir",
                str(upstream),
            ]
            supervisor = FakeSupervisor(self)
            options = with_zig.parse_arguments(arguments)
            with (
                mock.patch.object(with_zig, "host_spec", return_value=object()),
                mock.patch.object(
                    with_zig,
                    "provisioned_zig",
                    side_effect=fake_provisioned_zig,
                ),
            ):
                self.assertEqual(with_zig.run_benchmark(options, supervisor), 0)
            self.assertFalse(live)
            self.assertEqual(supervisor.calls, 2)


if __name__ == "__main__":
    unittest.main()
