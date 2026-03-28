use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_with_input(json: &str) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cmd-guard"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn cmd-guard");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(json.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let code = output.status.code().unwrap_or(-1);
    (stdout, code)
}

fn hook_json(command: &str) -> String {
    serde_json::json!({
        "tool_input": {
            "command": command
        }
    })
    .to_string()
}

fn run_setup(home: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cmd-guard"))
        .arg("--setup")
        .env("HOME", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to spawn cmd-guard")
}

// --- --version ---

#[test]
fn version_flag_prints_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_cmd-guard"))
        .arg("--version")
        .output()
        .expect("failed to spawn cmd-guard");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("cmd-guard {}", env!("CARGO_PKG_VERSION"))
    );
}

// --- --setup ---

#[test]
fn setup_creates_symlink_and_settings() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path();

    let output = run_setup(home);
    assert!(output.status.success());

    // Symlink exists and points to the test binary
    let symlink = home.join(".claude/hooks/cmd-guard");
    assert!(symlink.symlink_metadata().is_ok(), "symlink should exist");
    let target = std::fs::read_link(&symlink).expect("read_link");
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_cmd-guard"))
        .canonicalize()
        .unwrap();
    assert_eq!(target, exe);

    // settings.json contains the hook entry
    let settings_raw =
        std::fs::read_to_string(home.join(".claude/settings.json")).expect("read settings");
    let settings: serde_json::Value = serde_json::from_str(&settings_raw).expect("parse json");
    let hooks = settings
        .pointer("/hooks/PreToolUse")
        .expect("PreToolUse key");
    assert!(hooks.is_array());
    assert_eq!(hooks.as_array().unwrap().len(), 1);
    let cmd = hooks.pointer("/0/hooks/0/command").unwrap();
    assert!(cmd.as_str().unwrap().contains("cmd-guard"));
}

#[test]
fn setup_is_idempotent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path();

    run_setup(home);
    let output = run_setup(home);
    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("already"));

    // Still only one hook entry
    let settings_raw =
        std::fs::read_to_string(home.join(".claude/settings.json")).expect("read settings");
    let settings: serde_json::Value = serde_json::from_str(&settings_raw).expect("parse json");
    let hooks = settings
        .pointer("/hooks/PreToolUse")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(hooks.len(), 1);
}

#[test]
fn setup_preserves_existing_settings() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path();

    // Pre-populate settings.json with existing content
    let claude_dir = home.join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    let existing = serde_json::json!({
        "permissions": {"allow": ["Read"]},
        "hooks": {
            "PreToolUse": [{
                "matcher": "Write",
                "hooks": [{"type": "command", "command": "/usr/bin/other-hook"}]
            }]
        }
    });
    std::fs::write(
        claude_dir.join("settings.json"),
        serde_json::to_string_pretty(&existing).unwrap(),
    )
    .unwrap();

    let output = run_setup(home);
    assert!(output.status.success());

    let settings_raw =
        std::fs::read_to_string(claude_dir.join("settings.json")).expect("read settings");
    let settings: serde_json::Value = serde_json::from_str(&settings_raw).expect("parse json");

    // Existing keys preserved
    assert_eq!(settings["permissions"]["allow"][0], "Read");

    // Existing hook preserved, cmd-guard appended
    let hooks = settings["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(hooks.len(), 2);
    assert!(
        hooks[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("other-hook")
    );
    assert!(
        hooks[1]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("cmd-guard")
    );
}

// --- Pass-through (no output, exit 0) ---

#[test]
fn safe_command_passes_through() {
    let (stdout, code) = run_with_input(&hook_json("git status"));
    assert_eq!(code, 0);
    assert!(stdout.is_empty(), "safe commands should produce no output");
}

#[test]
fn safe_compound_passes_through() {
    let (stdout, code) = run_with_input(&hook_json("git status && git diff"));
    assert_eq!(code, 0);
    assert!(stdout.is_empty());
}

// --- Compound commands ---

#[test]
fn compound_deny_wins() {
    let (stdout, code) = run_with_input(&hook_json("git status && git push --force"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}

#[test]
fn compound_ask_wins() {
    let (stdout, code) = run_with_input(&hook_json("git status && git push origin main"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
}

// --- Deny verdicts ---

#[test]
fn deny_force_push() {
    let (stdout, code) = run_with_input(&hook_json("git push --force"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}

#[test]
fn deny_rm_rf() {
    let (stdout, code) = run_with_input(&hook_json("rm -rf /"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}

// --- Ask verdicts ---

#[test]
fn ask_plain_push() {
    let (stdout, code) = run_with_input(&hook_json("git push origin main"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
}

#[test]
fn ask_reset_hard() {
    let (stdout, code) = run_with_input(&hook_json("git reset --hard HEAD~1"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
}

// --- Output format ---

#[test]
fn output_has_correct_structure() {
    let (stdout, _) = run_with_input(&hook_json("git push --force"));
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let hso = &v["hookSpecificOutput"];
    assert_eq!(hso["hookEventName"], "PreToolUse");
    assert!(hso["permissionDecisionReason"].is_string());
    assert!(!hso["permissionDecisionReason"].as_str().unwrap().is_empty());
}

// --- Shell wrapper unwrapping ---

#[test]
fn deny_shell_wrapper_force_push() {
    let (stdout, code) = run_with_input(&hook_json("bash -c 'git push --force'"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}

// --- Docker/Podman ---

#[test]
fn deny_docker_system_prune_all() {
    let (stdout, code) = run_with_input(&hook_json("docker system prune -a"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}

#[test]
fn ask_podman_system_prune() {
    let (stdout, code) = run_with_input(&hook_json("podman system prune"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
}

// --- psql ---

#[test]
fn deny_psql_drop_database() {
    let (stdout, code) = run_with_input(&hook_json("psql -c DROP DATABASE mydb"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}

#[test]
fn ask_psql_drop_table() {
    let (stdout, code) = run_with_input(&hook_json("psql -c DROP TABLE users"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
}

// --- find ---

#[test]
fn deny_find_delete() {
    let (stdout, code) = run_with_input(&hook_json("find . -name '*.tmp' -delete"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}

// --- xargs ---

#[test]
fn deny_xargs_force_push() {
    let (stdout, code) = run_with_input(&hook_json("echo main | xargs git push --force"));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}

// --- Error handling (silent pass-through) ---

#[test]
fn malformed_json_passes_through() {
    let (stdout, code) = run_with_input("not json at all");
    assert_eq!(code, 0);
    assert!(stdout.is_empty());
}

#[test]
fn empty_input_passes_through() {
    let (stdout, code) = run_with_input("");
    assert_eq!(code, 0);
    assert!(stdout.is_empty());
}

#[test]
fn empty_command_passes_through() {
    let (stdout, code) = run_with_input(r#"{"tool_input":{"command":""}}"#);
    assert_eq!(code, 0);
    assert!(stdout.is_empty());
}

#[test]
fn missing_command_field_passes_through() {
    let (stdout, code) = run_with_input(r#"{"tool_input":{}}"#);
    assert_eq!(code, 0);
    assert!(stdout.is_empty());
}

#[test]
fn wrong_structure_passes_through() {
    let (stdout, code) = run_with_input(r#"{"foo":"bar"}"#);
    assert_eq!(code, 0);
    assert!(stdout.is_empty());
}
