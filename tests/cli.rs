use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn version_flag_prints_name_and_version() {
    Command::cargo_bin("arclite")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("arclite"));
}

#[test]
fn doctor_json_emits_valid_json() {
    let assert = Command::cargo_bin("arclite")
        .unwrap()
        .args(["doctor", "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert!(value["runtime"]["os"].is_string());
    assert!(value["tools"].is_object());
}

#[test]
fn inspect_json_reports_counts_and_manifest() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let assert = Command::cargo_bin("arclite")
        .unwrap()
        .args(["inspect", dir, "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert!(value["files"].as_u64().is_some());
    assert!(value["is_git_repo"].as_bool().unwrap());
    let manifests = value["manifests"].as_array().unwrap();
    assert!(manifests.iter().any(|m| m == "Cargo.toml"));
}
