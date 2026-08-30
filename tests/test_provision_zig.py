from __future__ import annotations

from contextlib import contextmanager, redirect_stderr
import hashlib
import io
from pathlib import Path
import stat
import tempfile
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

            def fake_extract(_archive: Path, destination: Path) -> None:
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

            def fake_extract(_archive: Path, destination: Path) -> None:
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

            def fake_extract(_archive: Path, destination: Path) -> None:
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

    def test_wrapper_binds_ephemeral_zig_and_forwards_arguments(self) -> None:
        zig = Path("/private/toolchain/zig")
        command = with_zig.upstream_command(zig, ["--runs", "30"])
        self.assertEqual(
            command,
            [
                with_zig.sys.executable,
                str(with_zig.ROOT / "benchmarks/upstream.py"),
                "--zig",
                str(zig),
                "--runs",
                "30",
            ],
        )

    def test_wrapper_validation_command_binds_live_exact_binaries(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "evidence.json"
            fx = root / "fx"
            machine = root / "machine-god"
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
                ]
            )
            self.assertEqual(
                with_zig.validation_command(options),
                [
                    with_zig.sys.executable,
                    str(with_zig.ROOT / "benchmarks/check.py"),
                    str(evidence),
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

    def test_wrapper_keeps_zig_live_through_collection_and_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fx = root / "fx"
            machine = root / "machine-god"
            fx.write_bytes(b"fx")
            machine.write_bytes(b"machine-god")
            live = False

            @contextmanager
            def fake_provisioned_zig(_cache: Path, _spec: object):
                nonlocal live
                live = True
                try:
                    yield root / "zig"
                finally:
                    live = False

            def fake_run(_command: list[str], *, check: bool) -> mock.Mock:
                self.assertTrue(live)
                self.assertFalse(check)
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
            ]
            with (
                mock.patch.object(with_zig, "host_spec", return_value=object()),
                mock.patch.object(
                    with_zig,
                    "provisioned_zig",
                    side_effect=fake_provisioned_zig,
                ),
                mock.patch.object(with_zig.subprocess, "run", side_effect=fake_run) as run,
            ):
                self.assertEqual(with_zig.main(arguments), 0)
            self.assertFalse(live)
            self.assertEqual(run.call_count, 2)


if __name__ == "__main__":
    unittest.main()
