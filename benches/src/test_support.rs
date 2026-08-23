use std::collections::BTreeMap;

use crate::manifest::Verification;
use crate::report::{
    BuildIdentity, CacheState, CorpusIdentity, Environment, Report, ReportInput, ResultInput,
    Sample, SessionConfig, ToolIdentity, REPORT_INPUT_SCHEMA,
};

pub(crate) fn verification(passed: bool) -> Verification {
    let digest = "a".repeat(64);
    Verification {
        passed,
        expected_manifest_digest: digest.clone(),
        actual_manifest_digest: if passed { digest } else { "b".repeat(64) },
        item_count: 10,
        logical_bytes: 100,
        mismatch_count: u64::from(!passed),
        mismatches: Vec::new(),
    }
}

fn sample(repetition: u32, order: u32, wall: f64) -> Sample {
    Sample {
        repetition,
        method_order: order,
        wall_seconds: wall,
        cpu_seconds: wall / 2.0,
        peak_rss_bytes: 1024,
        item_count: 10,
        logical_bytes: 100,
        wire_bytes: 80,
        phases_seconds: BTreeMap::from([("transfer".to_owned(), wall)]),
        cache_state: if repetition == 0 {
            CacheState::FirstPass
        } else {
            CacheState::Warm
        },
        cache_eviction_method: None,
        oracle: verification(true),
    }
}

pub(crate) fn report_input(candidate_wall: f64) -> ReportInput {
    let baseline_samples = (0..5)
        .map(|repetition| sample(repetition, u32::from(repetition % 2 == 0), 2.0))
        .collect();
    let candidate_samples = (0..5)
        .map(|repetition| sample(repetition, u32::from(repetition % 2 != 0), candidate_wall))
        .collect();
    ReportInput {
        schema: REPORT_INPUT_SCHEMA.to_owned(),
        build: BuildIdentity {
            source_revision: "abc123".to_owned(),
            build_id: "release-abc123".to_owned(),
            profile: "release".to_owned(),
        },
        environment: Environment {
            hardware: "machine".to_owned(),
            os: "os".to_owned(),
            kernel: "kernel".to_owned(),
            filesystem: "apfs".to_owned(),
            transport: "local".to_owned(),
            route: "local".to_owned(),
        },
        session: SessionConfig {
            streams: 1,
            compression: "none".to_owned(),
        },
        corpus: CorpusIdentity {
            schema: "fixture.v1".to_owned(),
            manifest_digest: "a".repeat(64),
            description: "test".to_owned(),
        },
        tools: vec![ToolIdentity {
            name: "xsync".to_owned(),
            version: "0.1.0".to_owned(),
            command: "xsync a b".to_owned(),
        }],
        results: vec![
            ResultInput {
                name: "baseline".to_owned(),
                baseline: None,
                samples: baseline_samples,
            },
            ResultInput {
                name: "candidate".to_owned(),
                baseline: Some("baseline".to_owned()),
                samples: candidate_samples,
            },
        ],
    }
}

pub(crate) fn report(candidate_wall: f64) -> Report {
    Report::from_input(report_input(candidate_wall)).unwrap()
}
