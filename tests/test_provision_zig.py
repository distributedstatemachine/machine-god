from __future__ import annotations

from contextlib import contextmanager, redirect_stderr
import fcntl
import hashlib
import io
import os
from pathlib import Path
import shutil
import signal
import stat
import sys
import tempfile
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

    def test_wrapper_sigterm_reaps_child_and_unwinds_zig_context(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            owned = root / "active/owned"

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
                    "import os,signal,time;"
                    "signal.signal(signal.SIGTERM,signal.SIG_IGN);"
                    "os.kill(os.getppid(),signal.SIGTERM);"
                    "time.sleep(60)"
                ),
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
                mock.patch.object(with_zig, "SIGNAL_GRACE_SECONDS", 0.1),
                mock.patch.object(with_zig, "KILL_GRACE_SECONDS", 1.0),
            ):
                exit_code = with_zig.main(
                    ["--cache-root", str(root / "cache"), "--", "--runs", "30"]
                )
            self.assertEqual(exit_code, 128 + signal.SIGTERM)
            self.assertFalse(owned.exists())
            self.assertLess(time.monotonic() - started, 3.0)

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
