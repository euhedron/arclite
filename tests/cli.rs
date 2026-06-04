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
