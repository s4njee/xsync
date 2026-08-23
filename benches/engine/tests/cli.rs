use std::fs;
use std::process::Command;

use tempfile::tempdir;
use xsync_engine_bench::report::EngineSample;

#[test]
fn isolated_worker_reports_scan_plan_and_queue_metrics() {
    let temp = tempdir().unwrap();
    for index in 0..32 {
        fs::write(temp.path().join(format!("file-{index:02}")), b"payload").unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_xsync-engine-bench"))
        .arg("worker")
        .arg("--root")
        .arg(temp.path())
        .arg("--channel-capacity")
        .arg("4")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let sample: EngineSample = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sample.item_count, 32);
    assert_eq!(sample.planned_items, 32);
    assert!(sample.scan_entries_per_second > 0.0);
    assert!(sample.queue_high_water <= 4);
}
