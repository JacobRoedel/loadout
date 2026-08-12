use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_loadout")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn loadout(path: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .arg(path)
        .env_remove("DATABASE_URL")
        .env_remove("OPTIONAL_TOKEN")
        .output()
        .expect("loadout binary runs")
}

fn temporary_repository() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loadout-cli-test-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory exists");
    root
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn check_json_filters_and_exit_codes_are_end_to_end() {
    let root = fixture("basic");
    let output = loadout(&root, &["check", "--only", "environment", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["schema_version"], "loadout/v1");
    assert_eq!(report["failed"], 1);
    assert_eq!(report["results"][0]["name"], "DATABASE_URL");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("node"));
}

#[test]
fn strict_turns_an_optional_environment_warning_into_a_failure() {
    let root = fixture("optional-env");
    let normal = loadout(&root, &["check", "--only", "environment", "--no-color"]);
    assert_eq!(normal.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&normal.stdout).contains("WARN"));

    let strict = loadout(
        &root,
        &["check", "--only", "environment", "--strict", "--no-color"],
    );
    assert_eq!(strict.status.code(), Some(1));
}

#[test]
fn doctor_reports_actionable_environment_blockers() {
    let root = fixture("basic");
    let output = loadout(
        &root,
        &["doctor", "--require", "env:DATABASE_URL", "--no-color"],
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Configure environment variables"));
    assert!(stdout.contains("DATABASE_URL"));
}

#[test]
fn changed_mode_keeps_only_requirements_from_modified_files() {
    let root = temporary_repository();
    fs::write(root.join(".env.example"), "DATABASE_URL=\n").unwrap();
    fs::write(
        root.join("package.json"),
        "{\"engines\": {\"node\": \">=20\"}}\n",
    )
    .unwrap();
    git(&root, &["init", "--quiet"]);
    git(&root, &["config", "user.email", "tests@example.com"]);
    git(&root, &["config", "user.name", "Loadout tests"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    git(&root, &["branch", "base"]);
    fs::write(root.join(".env.example"), "DATABASE_URL=\nAPI_KEY=\n").unwrap();
    git(&root, &["add", ".env.example"]);
    git(&root, &["commit", "--quiet", "-m", "env change"]);

    let output = loadout(
        &root,
        &[
            "check",
            "--changed",
            "base",
            "--only",
            "environment",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["results"].as_array().unwrap().len(), 2);
    fs::remove_dir_all(root).unwrap();
}
