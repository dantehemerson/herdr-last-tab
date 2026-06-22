use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "herdr-last-workspace-{label}-{}-{}",
            std::process::id(),
            current_unix_ms()
        ));
        fs::create_dir_all(&path).expect("temp directory should be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
#[cfg(unix)]
fn toggles_between_last_two_workspaces_by_id() {
    let temp = TempDir::new("toggle-cli");
    let state_dir = temp.path().join("state");
    let workspaces_path = temp.path().join("workspaces.json");
    let focus_log_path = temp.path().join("focus.log");
    let herdr_path = write_fake_herdr(temp.path());

    write_workspace_list(
        &workspaces_path,
        &[("workspace-a", true), ("workspace-b", false)],
    );
    run_plugin(
        &herdr_path,
        &state_dir,
        &workspaces_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("workspace-a"))
    .assert_success();

    write_workspace_list(
        &workspaces_path,
        &[("workspace-b", true), ("workspace-a", false)],
    );
    run_plugin(
        &herdr_path,
        &state_dir,
        &workspaces_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("workspace-b"))
    .assert_success();

    write_workspace_list(
        &workspaces_path,
        &[("workspace-b", true), ("workspace-a", false)],
    );
    run_plugin(
        &herdr_path,
        &state_dir,
        &workspaces_path,
        &focus_log_path,
        "toggle",
    )
    .assert_success();
    assert_eq!(read_focus_log(&focus_log_path), vec!["workspace-a"]);

    write_workspace_list(
        &workspaces_path,
        &[("workspace-a", true), ("workspace-b", false)],
    );
    run_plugin(
        &herdr_path,
        &state_dir,
        &workspaces_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("workspace-a"))
    .assert_success();

    write_workspace_list(
        &workspaces_path,
        &[("workspace-a", true), ("workspace-b", false)],
    );
    run_plugin(
        &herdr_path,
        &state_dir,
        &workspaces_path,
        &focus_log_path,
        "toggle",
    )
    .assert_success();
    assert_eq!(
        read_focus_log(&focus_log_path),
        vec!["workspace-a", "workspace-b"]
    );
}

#[test]
#[cfg(unix)]
fn closed_remembered_workspace_clears_target_and_makes_toggle_noop() {
    let temp = TempDir::new("closed-cli");
    let state_dir = temp.path().join("state");
    let workspaces_path = temp.path().join("workspaces.json");
    let focus_log_path = temp.path().join("focus.log");
    let herdr_path = write_fake_herdr(temp.path());

    write_workspace_list(
        &workspaces_path,
        &[("workspace-a", true), ("workspace-b", false)],
    );
    run_plugin(
        &herdr_path,
        &state_dir,
        &workspaces_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("workspace-a"))
    .assert_success();

    write_workspace_list(
        &workspaces_path,
        &[("workspace-b", true), ("workspace-a", false)],
    );
    run_plugin(
        &herdr_path,
        &state_dir,
        &workspaces_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("workspace-b"))
    .assert_success();

    write_workspace_list(&workspaces_path, &[("workspace-b", true)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &workspaces_path,
        &focus_log_path,
        "closed",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("workspace-a"))
    .assert_success();
    run_plugin(
        &herdr_path,
        &state_dir,
        &workspaces_path,
        &focus_log_path,
        "toggle",
    )
    .assert_success();

    assert!(read_focus_log(&focus_log_path).is_empty());
    assert_state(&state_dir, Some("workspace-b"), None);
}

#[test]
#[cfg(unix)]
fn malformed_state_file_is_replaced_without_focusing() {
    let temp = TempDir::new("malformed-cli");
    let state_dir = temp.path().join("state");
    let workspaces_path = temp.path().join("workspaces.json");
    let focus_log_path = temp.path().join("focus.log");
    let herdr_path = write_fake_herdr(temp.path());

    fs::create_dir_all(&state_dir).expect("state directory should be created");
    fs::write(state_dir.join("state.json"), "not json").expect("malformed state should write");
    write_workspace_list(&workspaces_path, &[("workspace-a", true)]);

    run_plugin(
        &herdr_path,
        &state_dir,
        &workspaces_path,
        &focus_log_path,
        "toggle",
    )
    .assert_success();

    assert!(read_focus_log(&focus_log_path).is_empty());
    assert_state(&state_dir, Some("workspace-a"), None);
}

#[test]
#[cfg(unix)]
fn concurrent_focused_events_keep_state_json_valid() {
    let temp = TempDir::new("concurrent-cli");
    let state_dir = temp.path().join("state");
    let focus_log_path = temp.path().join("focus.log");
    let herdr_path = write_fake_herdr(temp.path());
    let workspace_b_path = temp.path().join("workspaces-b.json");
    let workspace_c_path = temp.path().join("workspaces-c.json");

    write_workspace_list(
        &workspace_b_path,
        &[("workspace-b", true), ("workspace-c", false)],
    );
    write_workspace_list(
        &workspace_c_path,
        &[("workspace-c", true), ("workspace-b", false)],
    );

    let initial_path = temp.path().join("workspaces-a.json");
    write_workspace_list(&initial_path, &[("workspace-a", true)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &initial_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("workspace-a"))
    .assert_success();

    let handles = (0..12)
        .map(|index| {
            let herdr_path = herdr_path.clone();
            let state_dir = state_dir.clone();
            let focus_log_path = focus_log_path.clone();
            let workspace_path = if index % 2 == 0 {
                workspace_b_path.clone()
            } else {
                workspace_c_path.clone()
            };
            let event_workspace_id = if index % 2 == 0 {
                "workspace-b"
            } else {
                "workspace-c"
            };

            thread::spawn(move || {
                run_plugin(
                    &herdr_path,
                    &state_dir,
                    &workspace_path,
                    &focus_log_path,
                    "focused",
                )
                .env("HERDR_PLUGIN_EVENT_JSON", event_json(event_workspace_id))
                .assert_success();
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle.join().expect("focused event process should join");
    }

    let state = read_state_json(&state_dir);
    let current = state["current_workspace_id"]
        .as_str()
        .expect("current workspace id should be present");
    let last = state["last_workspace_id"]
        .as_str()
        .expect("last workspace id should be present");

    assert!(matches!(current, "workspace-b" | "workspace-c"));
    assert!(matches!(
        last,
        "workspace-a" | "workspace-b" | "workspace-c"
    ));
    assert_ne!(current, last);
}

#[cfg(unix)]
fn run_plugin(
    herdr_path: &Path,
    state_dir: &Path,
    workspaces_path: &Path,
    focus_log_path: &Path,
    subcommand: &str,
) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-last-workspace"));
    command
        .arg(subcommand)
        .env("HERDR_BIN_PATH", herdr_path)
        .env("HERDR_PLUGIN_STATE_DIR", state_dir)
        .env("FAKE_HERDR_WORKSPACE_LIST", workspaces_path)
        .env("FAKE_HERDR_FOCUS_LOG", focus_log_path);
    command
}

trait CommandAssertions {
    fn assert_success(&mut self);
}

impl CommandAssertions for Command {
    fn assert_success(&mut self) {
        let output = self.output().expect("plugin command should run");
        assert!(
            output.status.success(),
            "expected success, got status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
fn write_fake_herdr(root: &Path) -> PathBuf {
    let path = root.join("fake-herdr.sh");
    fs::write(
        &path,
        r#"#!/bin/sh
set -eu

if [ "$1" = "workspace" ] && [ "$2" = "list" ]; then
  cat "$FAKE_HERDR_WORKSPACE_LIST"
  exit 0
fi

if [ "$1" = "workspace" ] && [ "$2" = "focus" ]; then
  printf '%s\n' "$3" >> "$FAKE_HERDR_FOCUS_LOG"
  printf '{"id":"fake","result":{"type":"workspace_info","workspace":{"workspace_id":"%s","focused":true}}}\n' "$3"
  exit 0
fi

printf 'unexpected fake herdr command: %s %s\n' "${1:-}" "${2:-}" >&2
exit 64
"#,
    )
    .expect("fake Herdr script should write");

    let mut permissions = fs::metadata(&path)
        .expect("fake Herdr script metadata should load")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("fake Herdr script should be executable");
    path
}

fn write_workspace_list(path: &Path, workspaces: &[(&str, bool)]) {
    let workspaces = workspaces
        .iter()
        .enumerate()
        .map(|(index, (workspace_id, focused))| {
            serde_json::json!({
                "workspace_id": workspace_id,
                "number": index + 1,
                "label": workspace_id,
                "focused": focused,
                "pane_count": 1,
                "tab_count": 1,
                "active_tab_id": format!("{workspace_id}:t1"),
                "agent_status": "unknown"
            })
        })
        .collect::<Vec<_>>();
    let response = serde_json::json!({
        "id": "cli:workspace:list",
        "result": {
            "type": "workspace_list",
            "workspaces": workspaces
        }
    });
    fs::write(
        path,
        serde_json::to_string(&response).expect("workspace list should serialize"),
    )
    .expect("workspace list should write");
}

fn read_focus_log(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn assert_state(state_dir: &Path, current: Option<&str>, last: Option<&str>) {
    let state = read_state_json(state_dir);
    assert_eq!(state["current_workspace_id"].as_str(), current);
    assert_eq!(state["last_workspace_id"].as_str(), last);
}

fn read_state_json(state_dir: &Path) -> Value {
    let contents = fs::read_to_string(state_dir.join("state.json")).expect("state should exist");
    serde_json::from_str(&contents).expect("state should be valid JSON")
}

fn event_json(workspace_id: &str) -> String {
    format!(
        r#"{{"event":"workspace_focused","data":{{"type":"workspace_focused","workspace_id":"{workspace_id}"}}}}"#
    )
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
