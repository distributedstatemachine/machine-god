from pathlib import Path
import subprocess
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
CLI_MANIFEST = REPOSITORY_ROOT / "crates" / "machine-god-cli" / "Cargo.toml"
NON_WASM_CFG = 'cfg(not(target_family = "wasm"))'
SHA2_DEFAULT_NATIVE_CFG = 'cfg(any(target_os = "linux", target_os = "macos"))'
SHA2_WEB_FETCH_ONLY_NATIVE_CFG = (
    'cfg(all(not(target_family = "wasm"), '
    'not(any(target_os = "linux", target_os = "macos"))))'
)
MODEL_CATALOG_HTTP_DIRECT_DEPENDENCIES = {
    "hickory-proto",
    "hickory-resolver",
    "hyper",
    "reqwest",
    "rustls",
    "sha2",
    "tokio",
    "webpki-root-certs",
}


class NativeManifestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        with WORKSPACE_MANIFEST.open("rb") as manifest_file:
            cls.workspace_manifest = tomllib.load(manifest_file)
        with NATIVE_MANIFEST.open("rb") as manifest_file:
            cls.manifest = tomllib.load(manifest_file)
        with CLI_MANIFEST.open("rb") as manifest_file:
            cls.cli_manifest = tomllib.load(manifest_file)

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

    def test_web_fetch_sha2_is_optional_and_feature_gated(self) -> None:
        features = self.manifest["features"]
        self.assertIn("web-fetch-http", features)
        self.assertIn("web-fetch-http", features["ai-gateway-http"])
        self.assertEqual(
            [entry for entry in features["web-fetch-http"] if "sha2" in entry],
            ["dep:sha2"],
        )
        self.assertEqual(
            [
                entry
                for entry in features["ai-gateway-model-catalog-http"]
                if "sha2" in entry
            ],
            ["dep:sha2"],
        )

        target_tables = self.manifest["target"]
        non_wasm_dependencies = target_tables[NON_WASM_CFG]["dependencies"]
        feature_dependencies = {
            entry.removeprefix("dep:")
            for entry in features["web-fetch-http"]
            if entry.startswith("dep:")
        }
        self.assertLessEqual(
            feature_dependencies - {"sha2"},
            set(non_wasm_dependencies),
        )
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
                    "target",
                    SHA2_DEFAULT_NATIVE_CFG,
                    "sha2",
                    {"workspace": True},
                ),
                (
                    "target",
                    SHA2_WEB_FETCH_ONLY_NATIVE_CFG,
                    "sha2",
                    {"workspace": True, "optional": True},
                ),
            ],
        )

    def test_web_fetch_sha2_dependency_tree_is_feature_scoped(self) -> None:
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

        for target in ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]:
            self.assertIn("sha2", direct_dependencies(target))
            self.assertIn(
                "sha2",
                direct_dependencies(target, "--features", "web-fetch-http"),
            )

        for target in ["x86_64-unknown-freebsd", "x86_64-pc-windows-msvc"]:
            self.assertNotIn("sha2", direct_dependencies(target))
            self.assertIn(
                "sha2",
                direct_dependencies(target, "--features", "web-fetch-http"),
            )

        for feature_arguments in [(), ("--features", "web-fetch-http")]:
            self.assertNotIn(
                "sha2",
                direct_dependencies("wasm32-wasip1", *feature_arguments),
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


if __name__ == "__main__":
    unittest.main()
