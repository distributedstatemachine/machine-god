from pathlib import Path
import subprocess
import tomllib
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
NATIVE_MANIFEST = REPOSITORY_ROOT / "crates" / "machine-god-native" / "Cargo.toml"
NON_WASM_CFG = 'cfg(not(target_family = "wasm"))'
SHA2_DEFAULT_NATIVE_CFG = 'cfg(any(target_os = "linux", target_os = "macos"))'
SHA2_WEB_FETCH_ONLY_NATIVE_CFG = (
    'cfg(all(not(target_family = "wasm"), '
    'not(any(target_os = "linux", target_os = "macos"))))'
)


class NativeManifestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        with NATIVE_MANIFEST.open("rb") as manifest_file:
            cls.manifest = tomllib.load(manifest_file)

    def test_web_fetch_sha2_is_optional_and_feature_gated(self) -> None:
        features = self.manifest["features"]
        self.assertIn("web-fetch-http", features)
        self.assertIn("web-fetch-http", features["ai-gateway-http"])
        self.assertEqual(
            [entry for entry in features["web-fetch-http"] if "sha2" in entry],
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


if __name__ == "__main__":
    unittest.main()
