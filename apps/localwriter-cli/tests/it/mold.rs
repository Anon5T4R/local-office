//! The suite's agent-native mould in Local Writer: `commands --search`, `run --script` accepting a
//! step list or JSON Lines, and `--connect` taking a bare loopback port.

use std::process::{Command, Stdio};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_localwriter-cli");

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("localwriter-mould-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn json_of(out: &std::process::Output) -> Vec<Value> {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{:?}: {e}", String::from_utf8_lossy(&out.stdout)))
}

#[test]
fn commands_search_narrows_the_catalogue() {
    let all = Command::new(BIN).args(["commands", "--json"]).output().unwrap();
    assert!(all.status.success());
    let all = json_of(&all);
    let out = Command::new(BIN).args(["commands", "--json", "--search", "bold"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let found = json_of(&out);
    assert!(!found.is_empty() && found.len() < all.len(), "{} of {}", found.len(), all.len());
    for c in &found {
        let matched = ["id", "label", "location"].iter().any(|k| c[*k].as_str().is_some_and(|v| v.to_lowercase().contains("bold")));
        assert!(matched, "{} does not match `bold`", c["id"]);
    }
    // The text form filters the same way.
    let text = Command::new(BIN).args(["commands", "--search", "bold"]).output().unwrap();
    assert!(text.status.success());
    let text = String::from_utf8_lossy(&text.stdout).to_lowercase();
    assert!(text.contains("bold"), "{text}");
}

#[test]
fn a_script_may_be_one_step_per_line() {
    let script = tmp("job.jsonl");
    std::fs::write(&script, "{\"command\":\"text.insert\",\"params\":{\"text\":\"Hi\"}}\n\n\"format.bold\"\n").unwrap();
    let out_path = tmp("lines.txt");
    let out = Command::new(BIN).args(["run", "--script"]).arg(&script).arg("--save").arg(&out_path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(std::fs::read_to_string(&out_path).unwrap().contains("Hi"));
}

#[test]
fn a_script_may_be_a_list_of_steps() {
    let script = tmp("steps.json");
    std::fs::write(&script, "[{\"command\":\"text.insert\",\"params\":{\"text\":\"Hi\"}},\"format.bold\"]").unwrap();
    let out_path = tmp("steps.txt");
    let out = Command::new(BIN).args(["run", "--script"]).arg(&script).arg("--save").arg(&out_path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(std::fs::read_to_string(&out_path).unwrap().contains("Hi"));
}

#[test]
fn a_broken_line_names_its_step() {
    let script = tmp("broken.jsonl");
    std::fs::write(&script, "\"format.bold\"\n{oops}\n").unwrap();
    let out = Command::new(BIN).args(["run", "--script"]).arg(&script).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("step 2 (line 2)"), "{stderr}");
}

#[test]
fn connect_takes_a_bare_port_and_refuses_a_public_host() {
    // Bare port: the suite's `--connect PORT` means loopback. Nothing listens, so the bridge serves
    // lazily and ends when this client closes stdin.
    let out = Command::new(BIN).args(["mcp", "--connect", "1"]).stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // A host that isn't loopback never receives the control key, so the server refuses it.
    let out = Command::new(BIN).args(["mcp", "--connect", "192.168.1.5:7981"]).stdin(Stdio::null()).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("loopback only"));
}
