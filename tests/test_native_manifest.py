from pathlib import Path
import tomllib
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
NATIVE_MANIFEST = REPOSITORY_ROOT / "crates" / "machine-god-native" / "Cargo.toml"
NON_WASM_CFG = 'cfg(not(target_family = "wasm"))'


class NativeManifestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        with NATIVE_MANIFEST.open("rb") as manifest_file:
            cls.manifest = tomllib.load(manifest_file)

    def test_web_fetch_sha2_matches_non_wasm_feature_surface(self) -> None:
        features = self.manifest["features"]
        self.assertIn("web-fetch-http", features)
        self.assertIn("web-fetch-http", features["ai-gateway-http"])

        target_tables = self.manifest["target"]
        non_wasm_dependencies = target_tables[NON_WASM_CFG]["dependencies"]
        feature_dependencies = {
            entry.removeprefix("dep:")
            for entry in features["web-fetch-http"]
            if entry.startswith("dep:")
        }
        self.assertLessEqual(feature_dependencies, set(non_wasm_dependencies))
        self.assertEqual(non_wasm_dependencies["sha2"], {"workspace": True})

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
            [("target", NON_WASM_CFG, "sha2", {"workspace": True})],
        )


if __name__ == "__main__":
    unittest.main()
