import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("release-bench.py")
SPEC = importlib.util.spec_from_file_location("release_bench", MODULE_PATH)
release_bench = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules["release_bench"] = release_bench
SPEC.loader.exec_module(release_bench)


class ReleaseBenchHelpersTest(unittest.TestCase):
    def test_phase_timings_reads_timestamped_phase_events(self):
        output = "\n".join(
            json.dumps(event)
            for event in (
                {"event": "phase", "name": "scan", "started": True,
                 "timestamp_unix_nanos": 1_000_000_000},
                {"event": "phase", "name": "scan", "started": False,
                 "timestamp_unix_nanos": 1_500_000_000},
            )
        )
        self.assertEqual(release_bench.phase_timings(output), {"scan": 0.5})

    def test_real_mutation_is_reproducible_and_does_not_change_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            destination_one = root / "destination-one"
            destination_two = root / "destination-two"
            source.mkdir()
            for index in range(5):
                (source / f"file-{index}.txt").write_bytes(bytes([index + 1]) * 8)
            source_before = sorted(
                (path.relative_to(source).as_posix(), path.read_bytes())
                for path in source.rglob("*") if path.is_file()
            )

            _, selected_one = release_bench.seed_real_destination(
                source, destination_one, "content-churn", seed=42
            )
            _, selected_two = release_bench.seed_real_destination(
                source, destination_two, "content-churn", seed=42
            )

            self.assertEqual(selected_one, selected_two)
            self.assertNotEqual(
                (destination_one / selected_one[0]).read_bytes(),
                (source / selected_one[0]).read_bytes(),
            )
            source_after = sorted(
                (path.relative_to(source).as_posix(), path.read_bytes())
                for path in source.rglob("*") if path.is_file()
            )
            self.assertEqual(source_before, source_after)

    def test_destination_inside_corpus_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            corpus = release_bench.RealCorpus(
                "fixture", (root / "source",), root, "a" * 64
            )
            with self.assertRaisesRegex(release_bench.CellFailure, "fixture"):
                release_bench.validate_destination(root / "nested-destination", {"fixture": corpus})

    def test_registry_records_documented_counts(self):
        corpora = release_bench.real_corpora()
        self.assertEqual(corpora["congress-10k"].expected_file_count, 11_280)
        self.assertEqual(corpora["cb7"].expected_file_count, 204_577)

    def test_manifest_drift_reports_expected_and_observed_digests(self):
        corpus = release_bench.RealCorpus("fixture", (), Path("."), "a" * 64)
        with self.assertRaisesRegex(
            release_bench.DriftedCellFailure,
            "expected digest " + "a" * 64 + ".*observed " + "b" * 64,
        ):
            release_bench.validate_real_manifest(
                corpus, {"manifest_digest": "b" * 64, "entries": []}
            )

    def test_disposable_source_mutation_is_detected_between_manifests(self):
        bench = release_bench.REPO / "target/release/xsync-bench"
        if not bench.exists():
            self.skipTest("release xsync-bench binary is not built")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            file = source / "file.txt"
            file.write_bytes(b"before")
            corpus = release_bench.RealCorpus(
                "fixture", (source,), root, None, expected_file_count=1
            )
            first = release_bench.make_real_manifest(
                bench, corpus, root / "manifest-before.json"
            )
            corpus = release_bench.RealCorpus(
                "fixture", (source,), root, first["manifest_digest"],
                expected_file_count=1,
            )
            release_bench.validate_real_manifest(corpus, first)
            file.write_bytes(b"after")
            second = release_bench.make_real_manifest(
                bench, corpus, root / "manifest-after.json"
            )
            with self.assertRaises(release_bench.DriftedCellFailure):
                release_bench.validate_real_manifest(corpus, second)


if __name__ == "__main__":
    unittest.main()
