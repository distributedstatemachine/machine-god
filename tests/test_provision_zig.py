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

    def test_archive_validation_is_bounded_for_growth_endless_and_replacement(
        self,
    ) -> None:
        payload = b"pinned archive"
        spec = provision_zig.ToolchainSpec(
            "test-host", hashlib.sha256(payload).hexdigest(), len(payload)
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archive.tar.xz"
            archive.write_bytes(payload)
            real_read = provision_zig.os.read
            grew = False

            def grow_during_read(descriptor: int, requested: int) -> bytes:
                nonlocal grew
                chunk = real_read(descriptor, requested)
                if not grew:
                    grew = True
                    with archive.open("ab") as destination:
                        destination.write(b"growth")
                return chunk

            with mock.patch.object(
                provision_zig.os, "read", side_effect=grow_during_read
            ):
                self.assertIsNone(provision_zig.validated_archive(archive, spec))

            archive.write_bytes(payload)
            endless_reads = 0

            def endless_read(_descriptor: int, requested: int) -> bytes:
                nonlocal endless_reads
                endless_reads += 1
                return b"x" * requested

            with mock.patch.object(
                provision_zig.os, "read", side_effect=endless_read
            ):
                self.assertIsNone(provision_zig.validated_archive(archive, spec))
            self.assertEqual(endless_reads, 2)

            archive.write_bytes(payload)
            displaced = root / "displaced.tar.xz"
            replaced = False

            def replace_during_read(descriptor: int, requested: int) -> bytes:
                nonlocal replaced
                chunk = real_read(descriptor, requested)
                if not replaced:
                    replaced = True
                    archive.rename(displaced)
                    archive.write_bytes(payload)
                return chunk

            with mock.patch.object(
                provision_zig.os, "read", side_effect=replace_during_read
            ):
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

    def test_missing_archive_at_exact_capacity_never_starts_publication(self) -> None:
        payload = b"pinned archive"
        spec = provision_zig.ToolchainSpec(
            "test-host", hashlib.sha256(payload).hexdigest(), len(payload)
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archives = root / "archives"
            archives.mkdir()
            for index in range(provision_zig.ARCHIVE_DIRECTORY_CAPACITY):
                (archives / f"unrelated-{index:03d}").write_bytes(b"")

            with (
                mock.patch.object(provision_zig.tempfile, "mkstemp") as mkstemp,
                mock.patch.object(provision_zig, "download_archive") as download,
                self.assertRaisesRegex(provision_zig.ProvisionError, "capacity"),
            ):
                provision_zig.ensure_archive(root, spec)

            mkstemp.assert_not_called()
            download.assert_not_called()
            self.assertEqual(
                len(list(archives.iterdir())),
                provision_zig.ARCHIVE_DIRECTORY_CAPACITY,
            )

    def test_bounded_partial_prune_admits_exact_two_entry_publication(self) -> None:
        payload = b"pinned archive"
        spec = provision_zig.ToolchainSpec(
            "test-host", hashlib.sha256(payload).hexdigest(), len(payload)
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archives = root / "archives"
            archives.mkdir()
            for index in range(provision_zig.ARCHIVE_DIRECTORY_CAPACITY - 2):
                (archives / f"unrelated-{index:03d}").write_bytes(b"")
            for index in range(2):
                (archives / f".{spec.archive_name}.stale-{index}").write_bytes(
                    b"partial"
                )
            remaining = provision_zig.prune_partial_archives(root, spec)
            observed_counts: list[int] = []
            real_link = provision_zig.os.link

            def fake_download(destination: Path, _spec: object, _run: object) -> None:
                destination.write_bytes(payload)

            def observe_link(source: Path, destination: Path) -> None:
                observed_counts.append(len(list(archives.iterdir())))
                real_link(source, destination)
                observed_counts.append(len(list(archives.iterdir())))

            with (
                mock.patch.object(
                    provision_zig, "download_archive", side_effect=fake_download
                ),
                mock.patch.object(provision_zig.os, "link", side_effect=observe_link),
            ):
                archive = provision_zig.ensure_archive(
                    root,
                    spec,
                    known_archive_entries=remaining,
                )

            self.assertEqual(archive, (archives / spec.archive_name).resolve())
            self.assertEqual(
                observed_counts,
                [
                    provision_zig.ARCHIVE_DIRECTORY_CAPACITY - 1,
                    provision_zig.ARCHIVE_DIRECTORY_CAPACITY,
                ],
            )
            self.assertLessEqual(
                len(list(archives.iterdir())),
                provision_zig.ARCHIVE_DIRECTORY_CAPACITY,
            )

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

    def test_active_lock_contention_wait_keeps_termination_signals_unblocked(
        self,
    ) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archive.tar.xz"
            archive.write_bytes(b"fixture")
            real_cache_lock = provision_zig.cache_lock
            real_create_trash = provision_zig.create_private_trash
            wait_masks: list[set[signal.Signals]] = []
            mutation_masks: list[set[signal.Signals]] = []

            @contextmanager
            def observed_cache_lock(
                selected_root: Path,
                name: str = provision_zig.CACHE_LOCK_NAME,
            ):
                if name == provision_zig.ACTIVE_LOCK_NAME:
                    wait_masks.append(
                        set(signal.pthread_sigmask(signal.SIG_BLOCK, []))
                    )
                    time.sleep(0.02)
                with real_cache_lock(selected_root, name):
                    yield

            def observed_create_trash(selected_root: Path):
                mutation_masks.append(
                    set(signal.pthread_sigmask(signal.SIG_BLOCK, []))
                )
                return real_create_trash(selected_root)

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
                    "cache_lock",
                    side_effect=observed_cache_lock,
                ),
                mock.patch.object(
                    provision_zig,
                    "create_private_trash",
                    side_effect=observed_create_trash,
                ),
            ):
                with provision_zig.provisioned_zig(root, spec):
                    pass

            self.assertTrue(wait_masks)
            self.assertTrue(mutation_masks)
            for observed in wait_masks:
                self.assertTrue(
                    all(item not in observed for item in provision_zig.DEFERRED_SIGNALS)
                )
            for observed in mutation_masks:
                self.assertTrue(
                    all(item in observed for item in provision_zig.DEFERRED_SIGNALS)
                )

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

    def test_trash_claim_does_not_recreate_lease_after_owner_delete(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            trash = provision_zig.create_private_trash(root)
            os.close(trash.descriptor)
            real_open = provision_zig.open_existing_private_file
            interleaved = False

            def owner_delete_then_open(path: Path) -> int:
                nonlocal interleaved
                if not interleaved and path == trash.lease_path:
                    interleaved = True
                    shutil.rmtree(trash.path)
                    trash.lease_path.unlink()
                return real_open(path)

            with (
                provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME),
                mock.patch.object(
                    provision_zig,
                    "open_existing_private_file",
                    side_effect=owner_delete_then_open,
                ),
            ):
                claimed = provision_zig.claim_abandoned_trash(
                    root, [trash.path.name]
                )

            self.assertTrue(interleaved)
            self.assertEqual(claimed, [])
            self.assertFalse(trash.path.exists())
            self.assertFalse(trash.lease_path.exists())

    def test_trash_claim_rejects_directory_inode_replacement(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            trash = provision_zig.create_private_trash(root)
            os.close(trash.descriptor)
            displaced = root / "displaced"
            real_flock = provision_zig.fcntl.flock
            replaced = False

            def replace_after_lock(descriptor: int, operation: int) -> None:
                nonlocal replaced
                real_flock(descriptor, operation)
                if not replaced and operation & fcntl.LOCK_NB:
                    replaced = True
                    trash.path.rename(displaced)
                    trash.path.mkdir()

            with (
                provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME),
                mock.patch.object(
                    provision_zig.fcntl,
                    "flock",
                    side_effect=replace_after_lock,
                ),
                self.assertRaisesRegex(provision_zig.ProvisionError, "changed"),
            ):
                provision_zig.claim_abandoned_trash(root, [trash.path.name])

            self.assertTrue(replaced)
            shutil.rmtree(trash.path)
            shutil.rmtree(displaced)
            trash.lease_path.unlink()

    def test_trash_claim_closes_descriptor_at_every_post_open_fault(self) -> None:
        real_open = provision_zig.open_existing_private_file
        real_lstat = Path.lstat
        real_fstat = provision_zig.os.fstat

        for fault in (
            "flock",
            "lease_lstat",
            "fstat",
            "inode_mismatch",
            "directory_lstat",
            "append",
        ):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                trash = provision_zig.create_private_trash(root)
                os.close(trash.descriptor)
                opened: list[int] = []

                def capture_open(path: Path) -> int:
                    descriptor = real_open(path)
                    opened.append(descriptor)
                    return descriptor

                path_lstat_calls = 0

                def faulting_lstat(path: Path):
                    nonlocal path_lstat_calls
                    if fault == "lease_lstat" and path == trash.lease_path:
                        raise OSError("lease lstat failed")
                    if path == trash.path:
                        path_lstat_calls += 1
                        if fault == "directory_lstat" and path_lstat_calls == 2:
                            raise OSError("directory lstat failed")
                    return real_lstat(path)

                def faulting_fstat(descriptor: int):
                    if fault == "fstat":
                        raise OSError("fstat failed")
                    status = real_fstat(descriptor)
                    if fault == "inode_mismatch":
                        return mock.Mock(st_dev=status.st_dev, st_ino=status.st_ino + 1)
                    return status

                with provision_zig.cache_lock(
                    root, provision_zig.ACTIVE_LOCK_NAME
                ):
                    patches = [
                        mock.patch.object(
                            provision_zig,
                            "open_existing_private_file",
                            side_effect=capture_open,
                        ),
                        mock.patch.object(
                            Path,
                            "lstat",
                            autospec=True,
                            side_effect=faulting_lstat,
                        ),
                        mock.patch.object(
                            provision_zig.os, "fstat", side_effect=faulting_fstat
                        ),
                    ]
                    if fault == "flock":
                        patches.append(
                            mock.patch.object(
                                provision_zig.fcntl,
                                "flock",
                                side_effect=OSError("flock failed"),
                            )
                        )
                    if fault == "append":
                        patches.append(
                            mock.patch.object(
                                provision_zig,
                                "append_claimed_trash",
                                side_effect=MemoryError("append failed"),
                            )
                        )
                    for patch in patches:
                        patch.start()
                    try:
                        with self.assertRaises(BaseException):
                            provision_zig.claim_abandoned_trash(
                                root, [trash.path.name]
                            )
                    finally:
                        for patch in reversed(patches):
                            patch.stop()

                self.assertEqual(len(opened), 1)
                with self.assertRaises(OSError):
                    os.fstat(opened[0])

    def test_stale_active_rename_failure_closes_claim_descriptor(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            active = root / "active"
            active.mkdir()
            stale = active / "zig-stale"
            stale.mkdir()
            (stale / provision_zig.LEASE_NAME).write_text("stale", encoding="utf-8")
            trash = provision_zig.create_private_trash(root)
            opened: list[int] = []
            real_open = provision_zig.open_private_file

            def capture_open(path: Path, *, exclusive: bool = False) -> int:
                descriptor = real_open(path, exclusive=exclusive)
                if path == stale / provision_zig.LEASE_NAME:
                    opened.append(descriptor)
                return descriptor

            with (
                mock.patch.object(
                    provision_zig, "open_private_file", side_effect=capture_open
                ),
                mock.patch.object(Path, "rename", side_effect=OSError("rename failed")),
                self.assertRaisesRegex(OSError, "rename failed"),
            ):
                provision_zig.move_stale_active_to_trash(active, trash.path)

            self.assertEqual(len(opened), 1)
            with self.assertRaises(OSError):
                os.fstat(opened[0])
            provision_zig.remove_private_trash(trash)

    def test_orphan_trash_lease_is_retired_boundedly(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            trash = provision_zig.create_private_trash(root)
            shutil.rmtree(trash.path)
            os.close(trash.descriptor)

            with provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME):
                retired = provision_zig.retire_orphan_trash_leases(
                    root, [trash.lease_path.name]
                )

            self.assertEqual(retired, 1)
            self.assertFalse(trash.lease_path.exists())

    def test_bounded_directory_scan_stops_at_declared_witness(self) -> None:
        class FakeEntry:
            def __init__(self, name: str) -> None:
                self.name = name

        class CountingEntries:
            def __init__(self) -> None:
                self.inspected = 0

            def __enter__(self):
                return self

            def __exit__(self, *_args) -> None:
                return None

            def __iter__(self):
                return self

            def __next__(self) -> FakeEntry:
                self.inspected += 1
                return FakeEntry(f"entry-{self.inspected}")

        entries = CountingEntries()
        with (
            mock.patch.object(provision_zig.os, "scandir", return_value=entries),
            self.assertRaisesRegex(provision_zig.ProvisionError, "bounded"),
        ):
            provision_zig.bounded_directory_names(
                Path("ignored"),
                provision_zig.ACTIVE_DIRECTORY_SCAN_CAP,
                "fixture",
            )
        self.assertEqual(
            entries.inspected, provision_zig.ACTIVE_DIRECTORY_SCAN_CAP
        )

    def test_partial_archive_pruning_is_batched_within_hard_cap(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archives = root / "archives"
            archives.mkdir()
            count = provision_zig.PARTIAL_ARCHIVE_PRUNE_BATCH_SIZE + 2
            for index in range(count):
                (archives / f".{spec.archive_name}.{index:03d}").write_bytes(b"partial")

            provision_zig.prune_partial_archives(root, spec)
            self.assertEqual(
                len(list(archives.iterdir())),
                count - provision_zig.PARTIAL_ARCHIVE_PRUNE_BATCH_SIZE,
            )
            provision_zig.prune_partial_archives(root, spec)
            self.assertEqual(list(archives.iterdir()), [])

    def test_exact_root_capacity_refuses_trash_pair_without_exceeding_cap(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "archives").mkdir()
            (root / "active").mkdir()
            archive = root / "fixture.tar.xz"
            archive.write_bytes(b"fixture")
            with provision_zig.cache_lock(root):
                pass
            with provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME):
                pass
            for index in range(
                provision_zig.CACHE_DIRECTORY_CAPACITY - len(list(root.iterdir()))
            ):
                (root / f"filler-{index:03d}").write_bytes(b"")

            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                self.assertRaisesRegex(provision_zig.ProvisionError, "capacity"),
            ):
                with provision_zig.provisioned_zig(root, spec):
                    self.fail("an exact-capacity cache admitted another trash pair")

            self.assertEqual(
                len(list(root.iterdir())), provision_zig.CACHE_DIRECTORY_CAPACITY
            )

    def test_exact_root_capacity_creates_no_missing_infrastructure(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for index in range(provision_zig.CACHE_DIRECTORY_CAPACITY):
                (root / f"legacy-{index:03d}").write_bytes(b"")
            before = {entry.name for entry in root.iterdir()}

            with self.assertRaisesRegex(provision_zig.ProvisionError, "capacity"):
                with provision_zig.provisioned_zig(root, spec):
                    self.fail("an at-capacity root admitted fixed infrastructure")

            self.assertEqual({entry.name for entry in root.iterdir()}, before)
            self.assertFalse((root / provision_zig.CACHE_LOCK_NAME).exists())
            self.assertFalse((root / "archives").exists())
            self.assertFalse((root / "active").exists())

    def test_cursorless_trash_recovery_makes_bounded_progress_before_retry(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "archives").mkdir()
            (root / "active").mkdir()
            archive = root / "fixture.tar.xz"
            archive.write_bytes(b"fixture")
            with provision_zig.cache_lock(root):
                pass
            with provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME):
                pass
            for _index in range(33):
                trash = provision_zig.create_private_trash(root)
                os.close(trash.descriptor)
            filler = 0
            while len(list(root.iterdir())) < provision_zig.CACHE_DIRECTORY_CAPACITY - 1:
                (root / f"filler-{filler:03d}").write_bytes(b"")
                filler += 1
            before = len(list(root.iterdir()))
            self.assertFalse((root / provision_zig.TRASH_CURSOR_NAME).exists())

            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                self.assertRaisesRegex(provision_zig.ProvisionError, "retry"),
            ):
                with provision_zig.provisioned_zig(root, spec):
                    self.fail("bounded recovery unexpectedly admitted a run")

            self.assertLess(len(list(root.iterdir())), before)
            self.assertTrue((root / provision_zig.TRASH_CURSOR_NAME).is_file())
            self.assertLess(
                sum(
                    entry.name.startswith(provision_zig.TRASH_PREFIX)
                    for entry in root.iterdir()
                ),
                66,
            )

    def test_full_root_cursorless_recovery_rotates_past_live_window(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "archives").mkdir()
            (root / "active").mkdir()
            with provision_zig.cache_lock(root):
                pass
            with provision_zig.cache_lock(root, provision_zig.ACTIVE_LOCK_NAME):
                pass

            live_descriptors: list[int] = []
            for index in range(64):
                path = root / f"{provision_zig.TRASH_PREFIX}a{index:03d}"
                path.mkdir()
                descriptor = provision_zig.open_private_file(
                    provision_zig.trash_lease_path(path), exclusive=True
                )
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                live_descriptors.append(descriptor)
            abandoned = root / f"{provision_zig.TRASH_PREFIX}z-abandoned"
            abandoned.mkdir()
            abandoned_lease = provision_zig.open_private_file(
                provision_zig.trash_lease_path(abandoned), exclusive=True
            )
            os.close(abandoned_lease)
            filler = 0
            while len(list(root.iterdir())) < provision_zig.CACHE_DIRECTORY_CAPACITY:
                (root / f"filler-{filler:03d}").write_bytes(b"")
                filler += 1
            self.assertFalse((root / provision_zig.TRASH_CURSOR_NAME).exists())

            try:
                real_pwrite = os.pwrite
                for attempt in range(4):
                    if attempt == 1:
                        def interrupted_pwrite(
                            descriptor: int, payload: bytes, offset: int
                        ) -> int:
                            real_pwrite(
                                descriptor, payload[: len(payload) // 2], offset
                            )
                            raise OSError("interrupted cursor update")

                        with (
                            mock.patch.object(
                                provision_zig.os,
                                "pwrite",
                                side_effect=interrupted_pwrite,
                            ),
                            self.assertRaisesRegex(OSError, "interrupted"),
                            provision_zig.cache_lock(
                                root, provision_zig.ACTIVE_LOCK_NAME
                            ) as lock_descriptor,
                        ):
                            names = sorted(entry.name for entry in root.iterdir())
                            provision_zig.rotating_descriptor_trash_window(
                                lock_descriptor,
                                [
                                    name
                                    for name in names
                                    if name.startswith(provision_zig.TRASH_PREFIX)
                                ],
                            )
                        self.assertEqual(
                            len(list(root.iterdir())),
                            provision_zig.CACHE_DIRECTORY_CAPACITY,
                        )
                        continue
                    with provision_zig.cache_lock(
                        root, provision_zig.ACTIVE_LOCK_NAME
                    ) as lock_descriptor:
                        names = sorted(
                            provision_zig.bounded_directory_names(
                                root,
                                provision_zig.CACHE_DIRECTORY_SCAN_CAP,
                                "Zig cache root",
                            )
                        )
                        trash_names = [
                            name
                            for name in names
                            if name.startswith(provision_zig.TRASH_PREFIX)
                        ]
                        window = provision_zig.rotating_descriptor_trash_window(
                            lock_descriptor, trash_names
                        )
                        claimed = provision_zig.claim_abandoned_trash(root, window)
                    for trash in claimed:
                        provision_zig.remove_private_trash(trash)
                    self.assertLessEqual(
                        len(list(root.iterdir())),
                        provision_zig.CACHE_DIRECTORY_CAPACITY,
                    )
                    if not abandoned.exists():
                        break
            finally:
                for descriptor in live_descriptors:
                    os.close(descriptor)

            self.assertFalse(abandoned.exists())
            self.assertFalse(provision_zig.trash_lease_path(abandoned).exists())
            self.assertFalse((root / provision_zig.TRASH_CURSOR_NAME).exists())

    def test_cursor_records_preserve_last_valid_value_across_write_faults(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            standalone_name = "cursor"
            descriptor_path = root / "descriptor"
            descriptor = provision_zig.open_private_file(descriptor_path)
            real_pwrite = os.pwrite

            def standalone_write(value: str) -> None:
                provision_zig.write_scan_cursor(root, standalone_name, value)

            def standalone_read() -> str:
                return provision_zig.read_scan_cursor(root, standalone_name)

            def descriptor_write(value: str) -> None:
                provision_zig.write_descriptor_cursor(descriptor, value)

            def descriptor_read() -> str:
                return provision_zig.read_descriptor_cursor(descriptor)

            try:
                for label, write_cursor, read_cursor in (
                    ("standalone", standalone_write, standalone_read),
                    ("descriptor", descriptor_write, descriptor_read),
                ):
                    with self.subTest(storage=label, fault="post-truncate"):
                        write_cursor("stable")
                        with mock.patch.object(
                            provision_zig.os,
                            "ftruncate",
                            side_effect=AssertionError("must not truncate"),
                        ):
                            write_cursor("new-stable")
                        self.assertEqual(read_cursor(), "new-stable")

                    for fault in ("partial", "error", "interrupt", "torn"):
                        with self.subTest(storage=label, fault=fault):
                            stable = read_cursor()

                            def faulting_pwrite(
                                target: int, payload: bytes, offset: int
                            ) -> int:
                                if fault == "partial":
                                    return real_pwrite(
                                        target, payload[: len(payload) // 2], offset
                                    )
                                if fault == "error":
                                    raise OSError("ENOSPC")
                                if fault == "interrupt":
                                    raise KeyboardInterrupt("interrupted")
                                real_pwrite(target, payload[: len(payload) // 2], offset)
                                raise OSError("torn update")

                            expected = KeyboardInterrupt if fault == "interrupt" else OSError
                            with (
                                mock.patch.object(
                                    provision_zig.os,
                                    "pwrite",
                                    side_effect=faulting_pwrite,
                                ),
                                self.assertRaises(expected),
                            ):
                                write_cursor(f"failed-{fault}")
                            self.assertEqual(read_cursor(), stable)
            finally:
                os.close(descriptor)

    def test_concurrent_admission_uses_current_locked_active_count(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            active = root / "active"
            active.mkdir()
            existing = active / "zig-existing"
            existing.mkdir()
            existing_lease = provision_zig.open_private_file(
                existing / provision_zig.LEASE_NAME, exclusive=True
            )
            fcntl.flock(existing_lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
            archive = root / "fixture.tar.xz"
            archive.write_bytes(b"fixture")
            first_entered = threading.Event()
            release_first = threading.Event()
            errors: list[BaseException] = []
            observed_active_counts: list[int] = []
            real_create_active = provision_zig.create_active_lease

            def fake_extract(
                _archive: Path, destination: Path, _run: object = None
            ) -> None:
                destination.mkdir()
                executable = destination / "zig"
                executable.write_text(
                    "#!/bin/sh\nprintf '0.16.0\\n'\n", encoding="utf-8"
                )
                executable.chmod(executable.stat().st_mode | stat.S_IXUSR)

            def observe_create_active(
                selected_active: Path, selected_spec: provision_zig.ToolchainSpec
            ) -> tuple[Path, int]:
                result = real_create_active(selected_active, selected_spec)
                observed_active_counts.append(len(list(selected_active.iterdir())))
                return result

            def first_run() -> None:
                try:
                    with provision_zig.provisioned_zig(root, spec):
                        first_entered.set()
                        if not release_first.wait(timeout=5.0):
                            raise AssertionError("first run synchronization timed out")
                except BaseException as error:
                    errors.append(error)

            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                mock.patch.object(
                    provision_zig, "extract_archive", side_effect=fake_extract
                ),
                mock.patch.object(
                    provision_zig,
                    "create_active_lease",
                    side_effect=observe_create_active,
                ),
                mock.patch.object(provision_zig, "ACTIVE_DIRECTORY_CAPACITY", 2),
                mock.patch.object(provision_zig, "CACHE_DIRECTORY_CAPACITY", 64),
            ):
                first = threading.Thread(target=first_run)
                first.start()
                self.assertTrue(first_entered.wait(timeout=5.0))
                with self.assertRaisesRegex(provision_zig.ProvisionError, "capacity"):
                    with provision_zig.provisioned_zig(root, spec):
                        self.fail("stale pre-lock admission snapshot exceeded capacity")
                observed_active_counts.append(len(list(active.iterdir())))
                release_first.set()
                first.join(timeout=5.0)
                self.assertFalse(first.is_alive())

            os.close(existing_lease)
            shutil.rmtree(existing)
            self.assertEqual(errors, [])
            self.assertTrue(observed_active_counts)
            self.assertLessEqual(max(observed_active_counts), 2)

    def test_owned_cleanup_uses_but_never_exceeds_exact_root_reserve(self) -> None:
        spec = provision_zig.host_spec("Linux", "x86_64")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "fixture.tar.xz"
            archive.write_bytes(b"fixture")
            observed_root_counts: list[int] = []
            real_create_trash = provision_zig.create_private_trash

            def fake_extract(
                _archive: Path, destination: Path, _run: object = None
            ) -> None:
                destination.mkdir()
                executable = destination / "zig"
                executable.write_text(
                    "#!/bin/sh\nprintf '0.16.0\\n'\n", encoding="utf-8"
                )
                executable.chmod(executable.stat().st_mode | stat.S_IXUSR)

            context = provision_zig.provisioned_zig(root, spec)
            with (
                mock.patch.object(provision_zig, "ensure_archive", return_value=archive),
                mock.patch.object(
                    provision_zig, "extract_archive", side_effect=fake_extract
                ),
            ):
                context.__enter__()
            exact_capacity = len(list(root.iterdir())) + 2

            def observe_cleanup_trash(selected_root: Path):
                trash = real_create_trash(selected_root)
                observed_root_counts.append(len(list(selected_root.iterdir())))
                return trash

            with (
                mock.patch.object(
                    provision_zig, "CACHE_DIRECTORY_CAPACITY", exact_capacity
                ),
                mock.patch.object(
                    provision_zig,
                    "create_private_trash",
                    side_effect=observe_cleanup_trash,
                ),
            ):
                context.__exit__(None, None, None)

            self.assertEqual(observed_root_counts, [exact_capacity])
            self.assertLessEqual(len(list(root.iterdir())), exact_capacity)

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
            # Keep both group members as direct test children so their exit
            # status can be reaped. On Linux, kill(pid, 0) also succeeds for
            # a terminated orphan that container PID 1 has left as a zombie.
            leader = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(60)"],
                process_group=0,
            )
            worker = subprocess.Popen(
                [sys.executable, "-c", worker_source, str(worker_pid_path)],
                process_group=leader.pid,
            )
            try:
                deadline = time.monotonic() + 5.0
                while time.monotonic() < deadline and not worker_pid_path.exists():
                    time.sleep(0.01)
                self.assertTrue(worker_pid_path.exists())
                self.assertEqual(
                    int(worker_pid_path.read_text(encoding="utf-8")), worker.pid
                )

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
                worker.wait(timeout=2.0)
                self.assertEqual(worker.returncode, -signal.SIGKILL)
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
                try:
                    worker.kill()
                except ProcessLookupError:
                    pass
                worker.wait(timeout=1.0)

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
