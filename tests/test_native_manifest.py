import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
WORKSPACE_MANIFEST = REPOSITORY_ROOT / "Cargo.toml"
NATIVE_MANIFEST = REPOSITORY_ROOT / "crates" / "machine-god-native" / "Cargo.toml"
MODEL_CATALOG_HTTP_SOURCE = (
    REPOSITORY_ROOT
    / "crates"
    / "machine-god-native"
    / "src"
    / "ai_gateway_model_catalog_http.rs"
)
NATIVE_LIB_SOURCE = (
    REPOSITORY_ROOT / "crates" / "machine-god-native" / "src" / "lib.rs"
)
WEB_SEARCH_SOURCE = (
    REPOSITORY_ROOT
    / "crates"
    / "machine-god-native"
    / "src"
    / "web_search.rs"
)
VISION_PORTABLE_SOURCE = (
    REPOSITORY_ROOT
    / "crates"
    / "machine-god-native"
    / "src"
    / "vision_portable.rs"
)
VISION_DOCUMENT = REPOSITORY_ROOT / "docs" / "vision.md"
CI_WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"
CLI_MANIFEST = REPOSITORY_ROOT / "crates" / "machine-god-cli" / "Cargo.toml"
NON_WASM_CFG = 'cfg(not(target_family = "wasm"))'
VISION_ADAPTER_CFG = (
    'cfg(all(feature = "vision", not(target_family = "wasm")))'
)
VISION_TOOL_CFG = (
    'cfg(all(feature = "vision", not(target_family = "wasm")))'
)
VISION_REFERENCE_HOST_CFG = '''cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))'''
MODEL_CATALOG_HTTP_DIRECT_DEPENDENCIES = {
    "hickory-proto",
    "hickory-resolver",
    "hyper",
    "reqwest",
    "rustls",
    "tokio",
    "webpki-root-certs",
}
RELEASE_PANIC_PROBE = "ask_user_question_release_panic_probe"
RELEASE_PANIC_PROBE_SOURCE = (
    REPOSITORY_ROOT
    / "crates"
    / "machine-god-native"
    / "examples"
    / f"{RELEASE_PANIC_PROBE}.rs"
)
RELEASE_PANIC_PROBE_STDOUT = (
    b"ordinary-primary=prompt-drop\n"
    b"ambient-primary=ambient-drop\n"
    b"secondary-payload-drop=panics\n"
    b"secondary-payloads=suppressed\n"
    b"stale-target-wakes=0\n"
    b"target-drops=2 secondary-callbacks=2\n"
    b"fresh-capacity=2\n"
)


class NativeManifestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        with WORKSPACE_MANIFEST.open("rb") as manifest_file:
            cls.workspace_manifest = tomllib.load(manifest_file)
        with NATIVE_MANIFEST.open("rb") as manifest_file:
            cls.manifest = tomllib.load(manifest_file)
        with CLI_MANIFEST.open("rb") as manifest_file:
            cls.cli_manifest = tomllib.load(manifest_file)

    def test_release_profile_unwinds_owned_cleanup_panics(self) -> None:
        self.assertEqual(
            self.workspace_manifest["profile"]["release"]["panic"],
            "unwind",
        )

    def test_release_question_cleanup_probe_recovers_capacity(self) -> None:
        self.assertEqual(len(RELEASE_PANIC_PROBE_STDOUT), 193)
        source = RELEASE_PANIC_PROBE_SOURCE.read_text(encoding="utf-8")
        for required_fragment in [
            "struct PromptDropPanicFuture",
            "struct SecondaryTargetWithPromptWakerPanic",
            "Callback::Drop",
            "PrimaryCase::PromptDrop",
            "PrimaryCase::AmbientDrop",
            "ExecutionDropGuard(Some(execution))",
            "secondary_payload_drops.load(Ordering::SeqCst), 0",
            "closed_waker.wake_by_ref()",
        ]:
            self.assertIn(required_fragment, source)
        self.assertNotIn("struct PanickingPrompt", source)

        with tempfile.TemporaryDirectory(
            prefix="machine-god-release-panic-"
        ) as target_directory:
            environment = os.environ.copy()
            environment["CARGO_TARGET_DIR"] = target_directory
            environment.pop("CARGO_BUILD_TARGET", None)

            build = subprocess.run(
                [
                    "cargo",
                    "+1.94.1",
                    "build",
                    "--locked",
                    "--offline",
                    "--release",
                    "-p",
                    "machine-god-native",
                    "--example",
                    RELEASE_PANIC_PROBE,
                ],
                cwd=REPOSITORY_ROOT,
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=600,
            )
            self.assertEqual(
                build.returncode,
                0,
                msg=f"release probe build failed:\n{build.stderr[-4_000:]}",
            )

            executable = (
                Path(target_directory)
                / "release"
                / "examples"
                / f"{RELEASE_PANIC_PROBE}{'.exe' if os.name == 'nt' else ''}"
            )
            completed = subprocess.run(
                [str(executable)],
                cwd=REPOSITORY_ROOT,
                env=environment,
                check=False,
                capture_output=True,
                timeout=10,
            )
            self.assertLessEqual(len(completed.stdout), 256)
            self.assertLessEqual(len(completed.stderr), 256)
            self.assertEqual(
                completed.returncode,
                0,
                msg=(
                    "release panic probe failed: "
                    f"stdout={completed.stdout!r}, stderr={completed.stderr!r}"
                ),
            )
            self.assertEqual(
                completed.stdout,
                RELEASE_PANIC_PROBE_STDOUT,
            )
            self.assertEqual(completed.stderr, b"")

    def test_tokio_signal_feature_is_cli_only(self) -> None:
        workspace_tokio = self.workspace_manifest["workspace"]["dependencies"][
            "tokio"
        ]
        self.assertNotIn("signal", workspace_tokio["features"])

        cli_tokio = self.cli_manifest["target"][NON_WASM_CFG]["dependencies"][
            "tokio"
        ]
        self.assertTrue(cli_tokio["workspace"])
        self.assertEqual(cli_tokio["features"], ["signal"])

    def test_sha2_is_unconditional_for_terminal_environment_identity(self) -> None:
        features = self.manifest["features"]
        self.assertIn("web-fetch-http", features)
        self.assertIn("web-fetch-http", features["ai-gateway-http"])
        self.assertFalse(
            any(
                "sha2" in entry
                for feature in features.values()
                for entry in feature
            )
        )

        target_tables = self.manifest["target"]
        non_wasm_dependencies = target_tables[NON_WASM_CFG]["dependencies"]
        feature_dependencies = {
            entry.removeprefix("dep:")
            for entry in features["web-fetch-http"]
            if entry.startswith("dep:")
        }
        self.assertLessEqual(feature_dependencies, set(non_wasm_dependencies))
        self.assertNotIn("sha2", non_wasm_dependencies)

        dependency_tables = [
            ("dependencies", None, self.manifest.get("dependencies", {}))
        ]
        for target_cfg, target_table in target_tables.items():
            dependency_tables.append(
                ("target", target_cfg, target_table.get("dependencies", {}))
            )

        sha2_placements = [
            (surface, target_cfg, dependency_name, dependency_spec)
            for surface, target_cfg, dependencies in dependency_tables
            for dependency_name, dependency_spec in dependencies.items()
            if dependency_name == "sha2"
            or (
                isinstance(dependency_spec, dict)
                and dependency_spec.get("package") == "sha2"
            )
        ]
        self.assertEqual(
            sha2_placements,
            [
                (
                    "dependencies",
                    None,
                    "sha2",
                    {"workspace": True},
                ),
            ],
        )

    def test_terminal_sha2_dependency_tree_is_target_and_feature_neutral(self) -> None:
        def direct_dependencies(
            target: str, *feature_arguments: str
        ) -> set[str]:
            completed = subprocess.run(
                [
                    "cargo",
                    "tree",
                    "--locked",
                    "-p",
                    "machine-god-native",
                    "--edges",
                    "normal",
                    "--depth",
                    "1",
                    "--prefix",
                    "none",
                    "--format",
                    "{p}",
                    "--no-default-features",
                    "--target",
                    target,
                    *feature_arguments,
                ],
                cwd=REPOSITORY_ROOT,
                check=True,
                capture_output=True,
                text=True,
            )
            return {
                line.split(maxsplit=1)[0]
                for line in completed.stdout.splitlines()[1:]
                if line
            }

        for target in [
            "x86_64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-unknown-freebsd",
            "x86_64-pc-windows-msvc",
            "wasm32-wasip1",
        ]:
            for feature_arguments in [(), ("--features", "web-fetch-http")]:
                self.assertIn(
                    "sha2",
                    direct_dependencies(target, *feature_arguments),
                )

    def test_model_catalog_http_feature_omits_web_fetch_dependencies(self) -> None:
        features = self.manifest["features"]
        workspace_reqwest = self.workspace_manifest["workspace"]["dependencies"][
            "reqwest"
        ]
        self.assertNotIn("hickory-dns", workspace_reqwest["features"])
        self.assertEqual(
            set(features["ai-gateway-model-catalog-http"]),
            {
                f"dep:{dependency}"
                for dependency in MODEL_CATALOG_HTTP_DIRECT_DEPENDENCIES
            },
        )
        self.assertNotIn(
            "reqwest/hickory-dns", features["ai-gateway-model-catalog-http"]
        )
        self.assertEqual(
            set(features["ai-gateway-http"]),
            {
                "ai-gateway-model-catalog-http",
                "dep:bytes",
                "vision",
                "web-fetch-http",
            },
        )
        self.assertNotIn(
            "web-fetch-http", features["ai-gateway-model-catalog-http"]
        )

        cargo_tree_command = [
            "cargo",
            "tree",
            "--locked",
            "-p",
            "machine-god-native",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
            "--no-default-features",
            "--features",
            "ai-gateway-model-catalog-http",
        ]
        completed = subprocess.run(
            [*cargo_tree_command, "--depth", "1"],
            cwd=REPOSITORY_ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        direct_dependencies = {
            line.split(maxsplit=1)[0]
            for line in completed.stdout.splitlines()[1:]
            if line
        }
        self.assertLessEqual(
            MODEL_CATALOG_HTTP_DIRECT_DEPENDENCIES, direct_dependencies
        )
        self.assertIn("hickory-resolver", direct_dependencies)
        self.assertIn("hickory-proto", direct_dependencies)

        completed = subprocess.run(
            cargo_tree_command,
            cwd=REPOSITORY_ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        resolved_dependencies = {
            line.split(maxsplit=1)[0]
            for line in completed.stdout.splitlines()[1:]
            if line
        }
        self.assertLessEqual(
            {"hickory-net", "hickory-proto", "hickory-resolver", "moka"},
            resolved_dependencies,
        )
        self.assertNotIn("signal-hook-registry", resolved_dependencies)

    def test_model_catalog_android_dns_fails_closed_without_platform_api(self) -> None:
        source = MODEL_CATALOG_HTTP_SOURCE.read_text(encoding="utf-8")
        android_cfg = '#[cfg(target_os = "android")]\n'
        apple_windows_cfg = (
            '#[cfg(any(target_os = "windows", target_vendor = "apple"))]\n'
        )
        generic_unix_cfg = (
            '#[cfg(all(unix, not(any(target_os = "android", '
            'target_vendor = "apple"))))]\n'
        )
        loader_signature = "fn load_system_resolver_snapshot()\n"

        self.assertEqual(source.count(generic_unix_cfg + loader_signature), 1)
        self.assertEqual(source.count(android_cfg + loader_signature), 1)
        self.assertEqual(source.count(apple_windows_cfg + loader_signature), 1)

        android_branch = source.split(
            android_cfg + loader_signature, maxsplit=1
        )[1].split(
            "\n#[cfg(", maxsplit=1
        )[0]
        self.assertIn(
            "Err(SystemResolverConfigurationUnavailable)",
            android_branch,
        )
        self.assertNotIn("read_system_conf", android_branch)

        apple_windows_branch = source.split(
            apple_windows_cfg + loader_signature, maxsplit=1
        )[1].split("\n#[cfg(", maxsplit=1)[0]
        self.assertIn("read_system_conf", apple_windows_branch)

    def test_web_search_contract_is_target_neutral_and_http_tool_is_feature_scoped(self) -> None:
        lib_source = NATIVE_LIB_SOURCE.read_text(encoding="utf-8")
        web_search_source = WEB_SEARCH_SOURCE.read_text(encoding="utf-8")

        self.assertEqual(lib_source.count("\nmod web_search;\n"), 1)
        self.assertEqual(
            lib_source.count(
                '#[cfg(all(feature = "ai-gateway-http", '
                'not(target_family = "wasm")))]\n'
                "pub use web_search::WebSearchTool;"
            ),
            1,
        )
        self.assertIn("WebSearchDeadline, WebSearchLimits", lib_source)
        self.assertIn("pub trait WebSearchTransport", web_search_source)
        self.assertIn("pub trait WebSearchDeadline", web_search_source)
        self.assertEqual(web_search_source.count("pub struct WebSearchTool"), 1)
        self.assertNotIn("tokio::time", web_search_source)

        def normal_dependencies(target: str) -> set[str]:
            completed = subprocess.run(
                [
                    "cargo",
                    "tree",
                    "--locked",
                    "-p",
                    "machine-god-native",
                    "--edges",
                    "normal",
                    "--prefix",
                    "none",
                    "--format",
                    "{p}",
                    "--no-default-features",
                    "--target",
                    target,
                ],
                cwd=REPOSITORY_ROOT,
                check=True,
                capture_output=True,
                text=True,
            )
            return {
                line.split(maxsplit=1)[0]
                for line in completed.stdout.splitlines()[1:]
                if line
            }

        for target in ["x86_64-unknown-linux-gnu", "wasm32-wasip1"]:
            dependencies = normal_dependencies(target)
            self.assertNotIn("tokio", dependencies)
            self.assertNotIn("reqwest", dependencies)
            self.assertNotIn("hickory-resolver", dependencies)

    def test_vision_portable_contract_and_native_edges_are_feature_scoped(
        self,
    ) -> None:
        lib_source = NATIVE_LIB_SOURCE.read_text(encoding="utf-8")
        portable_source = VISION_PORTABLE_SOURCE.read_text(encoding="utf-8")
        vision_document = VISION_DOCUMENT.read_text(encoding="utf-8")
        features = self.manifest["features"]

        self.assertEqual(lib_source.count("\nmod vision_portable;\n"), 1)
        self.assertEqual(lib_source.count("pub use vision_portable::{"), 1)
        module_prefix = lib_source[: lib_source.index("mod vision_portable;")]
        self.assertNotIn("#[cfg", module_prefix.rsplit(";", maxsplit=1)[1])
        export_prefix = lib_source[: lib_source.index("pub use vision_portable::{")]
        self.assertNotIn("#[cfg", export_prefix.rsplit("};", maxsplit=1)[1])
        self.assertIn("VisionDeadline", portable_source)
        self.assertIn("pub trait VisionDeadline", portable_source)
        self.assertIn("VisionDeadline, VisionImage", lib_source)
        for dependency in ["base64", "reqwest", "tokio"]:
            self.assertNotIn(dependency, portable_source)

        tool_cfg = f"#[{VISION_TOOL_CFG}]\n"
        self.assertEqual(lib_source.count(tool_cfg + "mod vision;"), 1)
        self.assertEqual(lib_source.count(tool_cfg + "pub use vision::{"), 1)
        self.assertIn(
            "contracts are available without `vision`,\n"
            "`ai-gateway-http`, HTTP, or Tokio, including on WebAssembly",
            vision_document,
        )
        self.assertIn(
            "The narrow\n"
            "`vision` feature enables only Base64 encoding and Tokio and does not enable an\n"
            "HTTP or TLS stack",
            vision_document,
        )
        self.assertIn(
            "`ai-gateway-http` includes this feature for reference-host\n"
            "composition",
            vision_document,
        )
        self.assertIn(
            "Other native operating systems return the\n"
            "fixed `UnsupportedPlatform` construction failure",
            vision_document,
        )
        self.assertNotIn(
            "`VisionTool` type is available on non-WebAssembly native targets",
            vision_document,
        )

        reference_host_cfg = f"#[{VISION_REFERENCE_HOST_CFG}]\n"
        self.assertEqual(
            lib_source.count(reference_host_cfg + "mod reference_host;"),
            1,
        )
        self.assertEqual(
            lib_source.count(reference_host_cfg + "pub use reference_host::{"),
            1,
        )

        adapter_cfg = f"#[{VISION_ADAPTER_CFG}]\n"
        self.assertEqual(
            lib_source.count(adapter_cfg + "mod ai_gateway_vision;"),
            1,
        )
        self.assertEqual(
            lib_source.count(adapter_cfg + "pub use ai_gateway_vision::{"),
            1,
        )

        self.assertEqual(set(features["vision"]), {"dep:base64", "dep:tokio"})
        self.assertEqual(features["ai-gateway-http"].count("vision"), 1)
        self.assertNotIn("dep:base64", features["ai-gateway-http"])
        self.assertEqual(
            [
                (feature, dependency)
                for feature, dependencies in features.items()
                for dependency in dependencies
                if "base64" in dependency
            ],
            [("vision", "dep:base64")],
        )
        self.assertEqual(
            self.manifest["dependencies"]["base64"],
            {"workspace": True, "optional": True, "features": ["alloc"]},
        )

        completed = subprocess.run(
            [
                "cargo",
                "+1.94.1",
                "tree",
                "--locked",
                "-p",
                "machine-god-native",
                "--edges",
                "normal",
                "--depth",
                "1",
                "--prefix",
                "none",
                "--format",
                "{p}",
                "--no-default-features",
                "--target",
                "wasm32-wasip1",
            ],
            cwd=REPOSITORY_ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        direct_dependencies = {
            line.split(maxsplit=1)[0]
            for line in completed.stdout.splitlines()[1:]
            if line
        }
        self.assertTrue(
            {"base64", "bytes", "reqwest", "tokio"}.isdisjoint(
                direct_dependencies
            )
        )

        workflow = CI_WORKFLOW.read_text(encoding="utf-8")
        unsupported_job = """  unsupported-native-tools:
    name: Unsupported native tools (FreeBSD)
    runs-on: ubuntu-24.04
"""
        install_command = (
            'rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal '
            "--component clippy --target x86_64-unknown-freebsd"
        )
        clippy_command = (
            'cargo +"${RUST_TOOLCHAIN}" clippy --locked '
            "-p machine-god-native --lib --test semantic_search_unsupported "
            "--test vision_unsupported "
            "--no-default-features "
            "--features vision --target x86_64-unknown-freebsd -- -D warnings"
        )
        self.assertEqual(workflow.count(unsupported_job), 1)
        self.assertEqual(workflow.count(install_command), 1)
        self.assertEqual(workflow.count(clippy_command), 1)
        self.assertIn(
            "CI cross-compiles and runs warnings-denied Clippy over the narrow\n"
            "feature's library plus the vision and semantic-search unsupported-platform\n"
            "integration tests for `x86_64-unknown-freebsd` with Rust 1.94.1",
            vision_document,
        )


if __name__ == "__main__":
    unittest.main()
