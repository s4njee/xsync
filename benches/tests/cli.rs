use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use tempfile::tempdir;
use xsync_bench::manifest::Verification;
use xsync_bench::report::{
    BuildIdentity, CacheState, CorpusIdentity, Environment, ReportInput, ResultInput, Sample,
    SessionConfig, ToolIdentity, REPORT_INPUT_SCHEMA,
};

fn bench() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xsync-bench"))
}

fn run(args: &[&str]) -> std::process::Output {
    bench().args(args).output().unwrap()
}

#[test]
fn manifest_and_verify_commands_detect_mutation() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("tree");
    let manifest = temp.path().join("manifest.json");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file"), b"original").unwrap();

    let created = run(&[
        "manifest",
        root.to_str().unwrap(),
        "--out",
        manifest.to_str().unwrap(),
    ]);
    assert!(created.status.success(), "{created:?}");

    let verified = run(&[
        "verify",
        root.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert!(verified.status.success(), "{verified:?}");

    fs::write(root.join("file"), b"mutated!").unwrap();
    let failed = run(&[
        "verify",
        root.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("manifest mismatch"));
}

#[test]
fn corpus_command_creates_owned_content_pinned_scenario() {
    let temp = tempdir().unwrap();
    let generated = run(&[
        "corpus",
        "--base",
        temp.path().to_str().unwrap(),
        "--class",
        "deep-small",
        "--workload",
        "content-churn",
        "--seed",
        "42",
        "--entry-count",
        "120",
    ]);
    assert!(generated.status.success(), "{generated:?}");
    let root = String::from_utf8(generated.stdout).unwrap();
    let root = std::path::Path::new(root.trim());
    assert!(root.join(".xsync-bench-owned").is_file());
    assert!(root.join("source.manifest.json").is_file());
    assert!(root.join("destination-initial.manifest.json").is_file());
    let scenario: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("scenario.json")).unwrap()).unwrap();
    assert_eq!(scenario["schema"], "xsync.corpus.scenario.v1");
    assert_eq!(scenario["parameters"]["entry_count"], 120);
    assert!(scenario["changed_entries"].as_u64().unwrap() > 0);
    assert_ne!(
        scenario["expected"]["digest"],
        scenario["initial_destination"]["digest"]
    );
}

#[test]
fn report_and_strict_gate_commands_emit_artifacts() {
    let temp = tempdir().unwrap();
    let input_path = temp.path().join("input.json");
    let report_path = temp.path().join("report.json");
    let markdown_path = temp.path().join("report.md");
    let gate_path = temp.path().join("gate.json");
    fs::write(&input_path, serde_json::to_vec_pretty(&input()).unwrap()).unwrap();

    let reported = run(&[
        "report",
        "--input",
        input_path.to_str().unwrap(),
        "--json",
        report_path.to_str().unwrap(),
        "--markdown",
        markdown_path.to_str().unwrap(),
    ]);
    assert!(reported.status.success(), "{reported:?}");
    assert!(fs::read_to_string(&markdown_path)
        .unwrap()
        .contains("## Repetitions"));

    let gated = run(&[
        "gate",
        "--current",
        report_path.to_str().unwrap(),
        "--baseline",
        report_path.to_str().unwrap(),
        "--strict",
        "--json",
        gate_path.to_str().unwrap(),
    ]);
    assert!(gated.status.success(), "{gated:?}");
    assert!(String::from_utf8_lossy(&gated.stdout).contains("1 comparison(s) performed"));
    assert!(gate_path.is_file());
}

fn input() -> ReportInput {
    let digest = "a".repeat(64);
    let verification = Verification {
        passed: true,
        expected_manifest_digest: digest.clone(),
        actual_manifest_digest: digest.clone(),
        item_count: 10,
        logical_bytes: 100,
        mismatch_count: 0,
        mismatches: Vec::new(),
    };
    let sample = |repetition: u32, method_order: u32, wall_seconds: f64| Sample {
        repetition,
        method_order,
        wall_seconds,
        cpu_seconds: wall_seconds / 2.0,
        peak_rss_bytes: 1024,
        item_count: 10,
        logical_bytes: 100,
        wire_bytes: 80,
        phases_seconds: BTreeMap::from([("transfer".to_owned(), wall_seconds)]),
        cache_state: if repetition == 0 {
            CacheState::FirstPass
        } else {
            CacheState::Warm
        },
        cache_eviction_method: None,
        oracle: verification.clone(),
    };
    ReportInput {
        schema: REPORT_INPUT_SCHEMA.to_owned(),
        build: BuildIdentity {
            source_revision: "revision".to_owned(),
            build_id: "release-revision".to_owned(),
            profile: "release".to_owned(),
        },
        environment: Environment {
            hardware: "test".to_owned(),
            os: "test-os".to_owned(),
            kernel: "test-kernel".to_owned(),
            filesystem: "test-fs".to_owned(),
            transport: "local".to_owned(),
            route: "local".to_owned(),
        },
        session: SessionConfig {
            streams: 1,
            compression: "none".to_owned(),
        },
        corpus: CorpusIdentity {
            schema: "fixture.v1".to_owned(),
            manifest_digest: digest,
            description: "integration".to_owned(),
        },
        tools: vec![ToolIdentity {
            name: "xsync".to_owned(),
            version: "0.1.0".to_owned(),
            command: "xsync source destination".to_owned(),
        }],
        results: vec![
            ResultInput {
                name: "baseline".to_owned(),
                baseline: None,
                samples: (0..5)
                    .map(|repetition| sample(repetition, u32::from(repetition % 2 == 0), 2.0))
                    .collect(),
            },
            ResultInput {
                name: "candidate".to_owned(),
                baseline: Some("baseline".to_owned()),
                samples: (0..5)
                    .map(|repetition| sample(repetition, u32::from(repetition % 2 != 0), 1.0))
                    .collect(),
            },
        ],
    }
}
