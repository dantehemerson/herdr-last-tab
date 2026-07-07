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
            "herdr-last-tab-{label}-{}-{}",
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
fn toggles_between_last_two_tabs_by_id() {
    let temp = TempDir::new("toggle-cli");
    let state_dir = temp.path().join("state");
    let tabs_path = temp.path().join("tabs.json");
    let focus_log_path = temp.path().join("focus.log");
    let herdr_path = write_fake_herdr(temp.path());

    write_tab_list(&tabs_path, &[("tab-a", true), ("tab-b", false)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &tabs_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("tab-a"))
    .assert_success();

    write_tab_list(&tabs_path, &[("tab-b", true), ("tab-a", false)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &tabs_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("tab-b"))
    .assert_success();

    write_tab_list(&tabs_path, &[("tab-b", true), ("tab-a", false)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &tabs_path,
        &focus_log_path,
        "toggle",
    )
    .assert_success();
    assert_eq!(read_focus_log(&focus_log_path), vec!["tab-a"]);

    write_tab_list(&tabs_path, &[("tab-a", true), ("tab-b", false)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &tabs_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("tab-a"))
    .assert_success();

    write_tab_list(&tabs_path, &[("tab-a", true), ("tab-b", false)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &tabs_path,
        &focus_log_path,
        "toggle",
    )
    .assert_success();
    assert_eq!(read_focus_log(&focus_log_path), vec!["tab-a", "tab-b"]);
}

#[test]
#[cfg(unix)]
fn closed_remembered_tab_clears_target_and_makes_toggle_noop() {
    let temp = TempDir::new("closed-cli");
    let state_dir = temp.path().join("state");
    let tabs_path = temp.path().join("tabs.json");
    let focus_log_path = temp.path().join("focus.log");
    let herdr_path = write_fake_herdr(temp.path());

    write_tab_list(&tabs_path, &[("tab-a", true), ("tab-b", false)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &tabs_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("tab-a"))
    .assert_success();

    write_tab_list(&tabs_path, &[("tab-b", true), ("tab-a", false)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &tabs_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("tab-b"))
    .assert_success();

    write_tab_list(&tabs_path, &[("tab-b", true)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &tabs_path,
        &focus_log_path,
        "closed",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("tab-a"))
    .assert_success();
    run_plugin(
        &herdr_path,
        &state_dir,
        &tabs_path,
        &focus_log_path,
        "toggle",
    )
    .assert_success();

    assert!(read_focus_log(&focus_log_path).is_empty());
    assert_state(&state_dir, Some("tab-b"), None);
}

#[test]
#[cfg(unix)]
fn malformed_state_file_is_replaced_without_focusing() {
    let temp = TempDir::new("malformed-cli");
    let state_dir = temp.path().join("state");
    let tabs_path = temp.path().join("tabs.json");
    let focus_log_path = temp.path().join("focus.log");
    let herdr_path = write_fake_herdr(temp.path());

    fs::create_dir_all(&state_dir).expect("state directory should be created");
    fs::write(state_dir.join("state.json"), "not json").expect("malformed state should write");
    write_tab_list(&tabs_path, &[("tab-a", true)]);

    run_plugin(
        &herdr_path,
        &state_dir,
        &tabs_path,
        &focus_log_path,
        "toggle",
    )
    .assert_success();

    assert!(read_focus_log(&focus_log_path).is_empty());
    assert_state(&state_dir, Some("tab-a"), None);
}

#[test]
#[cfg(unix)]
fn concurrent_focused_events_keep_state_json_valid() {
    let temp = TempDir::new("concurrent-cli");
    let state_dir = temp.path().join("state");
    let focus_log_path = temp.path().join("focus.log");
    let herdr_path = write_fake_herdr(temp.path());
    let tab_b_path = temp.path().join("tabs-b.json");
    let tab_c_path = temp.path().join("tabs-c.json");

    write_tab_list(&tab_b_path, &[("tab-b", true), ("tab-c", false)]);
    write_tab_list(&tab_c_path, &[("tab-c", true), ("tab-b", false)]);

    let initial_path = temp.path().join("tabs-a.json");
    write_tab_list(&initial_path, &[("tab-a", true)]);
    run_plugin(
        &herdr_path,
        &state_dir,
        &initial_path,
        &focus_log_path,
        "focused",
    )
    .env("HERDR_PLUGIN_EVENT_JSON", event_json("tab-a"))
    .assert_success();

    let handles = (0..12)
        .map(|index| {
            let herdr_path = herdr_path.clone();
            let state_dir = state_dir.clone();
            let focus_log_path = focus_log_path.clone();
            let tab_path = if index % 2 == 0 {
                tab_b_path.clone()
            } else {
                tab_c_path.clone()
            };
            let event_tab_id = if index % 2 == 0 { "tab-b" } else { "tab-c" };

            thread::spawn(move || {
                run_plugin(
                    &herdr_path,
                    &state_dir,
                    &tab_path,
                    &focus_log_path,
                    "focused",
                )
                .env("HERDR_PLUGIN_EVENT_JSON", event_json(event_tab_id))
                .assert_success();
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle.join().expect("focused event process should join");
    }

    let state = read_state_json(&state_dir);
    let current = state["current_tab_id"]
        .as_str()
        .expect("current tab id should be present");
    let last = state["last_tab_id"]
        .as_str()
        .expect("last tab id should be present");

    assert!(matches!(current, "tab-b" | "tab-c"));
    assert!(matches!(last, "tab-a" | "tab-b" | "tab-c"));
    assert_ne!(current, last);
}

#[cfg(unix)]
fn run_plugin(
    herdr_path: &Path,
    state_dir: &Path,
    tabs_path: &Path,
    focus_log_path: &Path,
    subcommand: &str,
) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-last-tab"));
    command
        .arg(subcommand)
        .env("HERDR_BIN_PATH", herdr_path)
        .env("HERDR_PLUGIN_STATE_DIR", state_dir)
        .env("FAKE_HERDR_TAB_LIST", tabs_path)
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

if [ "$1" = "tab" ] && [ "$2" = "list" ]; then
  cat "$FAKE_HERDR_TAB_LIST"
  exit 0
fi

if [ "$1" = "tab" ] && [ "$2" = "focus" ]; then
  printf '%s\n' "$3" >> "$FAKE_HERDR_FOCUS_LOG"
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

fn write_tab_list(path: &Path, tabs: &[(&str, bool)]) {
    let tabs = tabs
        .iter()
        .enumerate()
        .map(|(index, (tab_id, focused))| {
            serde_json::json!({
                "tab_id": tab_id,
                "workspace_id": "w1",
                "number": index + 1,
                "label": tab_id,
                "focused": focused,
                "pane_count": 1,
                "agent_status": "unknown"
            })
        })
        .collect::<Vec<_>>();
    let response = serde_json::json!({
        "id": "cli:tab:list",
        "result": {
            "type": "tab_list",
            "tabs": tabs
        }
    });
    fs::write(
        path,
        serde_json::to_string(&response).expect("tab list should serialize"),
    )
    .expect("tab list should write");
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
    assert_eq!(state["current_tab_id"].as_str(), current);
    assert_eq!(state["last_tab_id"].as_str(), last);
}

fn read_state_json(state_dir: &Path) -> Value {
    let contents = fs::read_to_string(state_dir.join("state.json")).expect("state should exist");
    serde_json::from_str(&contents).expect("state should be valid JSON")
}

fn event_json(tab_id: &str) -> String {
    format!(r#"{{"event":"tab_focused","data":{{"type":"tab_focused","tab_id":"{tab_id}"}}}}"#)
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
