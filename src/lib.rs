use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const STATE_FILE_NAME: &str = "state.json";
const LOCK_FILE_NAME: &str = "state.lock";

pub fn run_from_env() -> i32 {
    let subcommand = match parse_subcommand(env::args().skip(1)) {
        Ok(subcommand) => subcommand,
        Err(ParseCommandError::Usage(message)) => {
            eprintln!("{message}");
            return 2;
        }
    };

    let state_dir = match env_path("HERDR_PLUGIN_STATE_DIR") {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let herdr_bin_path = match env_path("HERDR_BIN_PATH") {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };

    let runtime = RuntimeEnv {
        state_dir,
        event_json: env::var("HERDR_PLUGIN_EVENT_JSON").ok(),
        context_json: env::var("HERDR_PLUGIN_CONTEXT_JSON").ok(),
    };
    let herdr = CliHerdr {
        bin_path: herdr_bin_path,
    };

    match run(subcommand, &runtime, &herdr) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn run(subcommand: Subcommand, runtime: &RuntimeEnv, herdr: &dyn Herdr) -> Result<(), PluginError> {
    match subcommand {
        Subcommand::Toggle => toggle(runtime, herdr),
        Subcommand::Focused => focused(runtime, herdr),
        Subcommand::Closed => closed(runtime, herdr),
    }
}

fn toggle(runtime: &RuntimeEnv, herdr: &dyn Herdr) -> Result<(), PluginError> {
    let snapshot = herdr.tab_list()?;
    let context_tab_id = runtime
        .context_json
        .as_deref()
        .and_then(parse_context_tab_id);
    let current_tab_id = snapshot
        .focused_tab_id()
        .map(str::to_owned)
        .or_else(|| context_tab_id.filter(|id| snapshot.contains_tab(id)));

    let store = StateStore::new(&runtime.state_dir);
    let Some(current_tab_id) = current_tab_id else {
        store.update(|memory| (memory, ()))?;
        return Ok(());
    };

    let target = store.update(|memory| match memory {
        Memory::Empty => (
            Memory::Current {
                current: current_tab_id.clone(),
                last: None,
            },
            None,
        ),
        Memory::Current {
            current,
            last: None,
        } => (
            Memory::Current {
                current,
                last: None,
            },
            None,
        ),
        Memory::Current {
            current,
            last: Some(last),
        } => {
            if !snapshot.contains_tab(&last) || last == current_tab_id {
                (
                    Memory::Current {
                        current,
                        last: None,
                    },
                    None,
                )
            } else {
                let target = last.clone();
                (
                    Memory::Current {
                        current,
                        last: Some(last),
                    },
                    Some(target),
                )
            }
        }
    })?;

    if let Some(target) = target {
        herdr.focus_tab(&target)?;
    }

    Ok(())
}

fn focused(runtime: &RuntimeEnv, herdr: &dyn Herdr) -> Result<(), PluginError> {
    let Some(event_tab_id) = runtime.event_json.as_deref().and_then(parse_event_tab_id) else {
        return Ok(());
    };

    let snapshot = herdr.tab_list()?;
    if snapshot.focused_tab_id() != Some(event_tab_id.as_str()) {
        return Ok(());
    }

    StateStore::new(&runtime.state_dir).update(|memory| {
        let next = match memory {
            Memory::Empty => Memory::Current {
                current: event_tab_id,
                last: None,
            },
            Memory::Current { current, last } if current == event_tab_id => {
                Memory::Current { current, last }
            }
            Memory::Current { current, .. } => Memory::Current {
                current: event_tab_id,
                last: Some(current),
            },
        };
        (next, ())
    })?;

    Ok(())
}

fn closed(runtime: &RuntimeEnv, herdr: &dyn Herdr) -> Result<(), PluginError> {
    let Some(closed_tab_id) = runtime.event_json.as_deref().and_then(parse_event_tab_id) else {
        return Ok(());
    };

    let snapshot = herdr.tab_list()?;
    let focused_tab_id = snapshot.focused_tab_id().map(str::to_owned);

    StateStore::new(&runtime.state_dir).update(|memory| {
        let next = match memory {
            Memory::Empty => Memory::Empty,
            Memory::Current { current, last } => {
                let current = if current == closed_tab_id {
                    focused_tab_id.clone()
                } else {
                    Some(current)
                };

                match current {
                    Some(current) => {
                        let last = last.filter(|last| last != &closed_tab_id && last != &current);
                        Memory::Current { current, last }
                    }
                    None => Memory::Empty,
                }
            }
        };
        (next, ())
    })?;

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subcommand {
    Toggle,
    Focused,
    Closed,
}

enum ParseCommandError {
    Usage(String),
}

fn parse_subcommand<I>(args: I) -> Result<Subcommand, ParseCommandError>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let Some(raw_subcommand) = args.next() else {
        return Err(ParseCommandError::Usage(usage()));
    };
    if args.next().is_some() {
        return Err(ParseCommandError::Usage(usage()));
    }

    match raw_subcommand.as_str() {
        "toggle" => Ok(Subcommand::Toggle),
        "focused" => Ok(Subcommand::Focused),
        "closed" => Ok(Subcommand::Closed),
        "help" | "--help" | "-h" => Err(ParseCommandError::Usage(usage())),
        other => Err(ParseCommandError::Usage(format!(
            "unknown subcommand: {other}\n{}",
            usage()
        ))),
    }
}

fn usage() -> String {
    "usage: herdr-last-tab <toggle|focused|closed>".to_string()
}

fn env_path(name: &str) -> Result<PathBuf, PluginError> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            PluginError::new(format!(
                "{name} is not set; Herdr should provide this environment variable to plugin commands"
            ))
        })
}

struct RuntimeEnv {
    state_dir: PathBuf,
    event_json: Option<String>,
    context_json: Option<String>,
}

trait Herdr {
    fn tab_list(&self) -> Result<TabSnapshot, PluginError>;
    fn focus_tab(&self, workspace_id: &str) -> Result<(), PluginError>;
}

struct CliHerdr {
    bin_path: PathBuf,
}

impl Herdr for CliHerdr {
    fn tab_list(&self) -> Result<TabSnapshot, PluginError> {
        let output = Command::new(&self.bin_path)
            .arg("tab")
            .arg("list")
            .output()
            .map_err(|error| {
                PluginError::new(format!(
                    "failed to run HERDR_BIN_PATH tab list ({}): {error}",
                    self.bin_path.display()
                ))
            })?;

        if !output.status.success() {
            return Err(command_failure("HERDR_BIN_PATH tab list", &output));
        }

        parse_tab_list_response(&output.stdout)
    }

    fn focus_tab(&self, tab_id: &str) -> Result<(), PluginError> {
        let output = Command::new(&self.bin_path)
            .arg("tab")
            .arg("focus")
            .arg(tab_id)
            .output()
            .map_err(|error| {
                PluginError::new(format!(
                    "failed to run HERDR_BIN_PATH tab focus {tab_id} ({}): {error}",
                    self.bin_path.display()
                ))
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(command_failure("HERDR_BIN_PATH tab focus", &output))
        }
    }
}

fn command_failure(command: &str, output: &Output) -> PluginError {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        output.status.to_string()
    };
    PluginError::new(format!("{command} failed: {detail}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TabSnapshot {
    tabs: Vec<TabInfo>,
}

impl TabSnapshot {
    fn focused_tab_id(&self) -> Option<&str> {
        self.tabs
            .iter()
            .find(|tab| tab.focused)
            .map(|tab| tab.tab_id.as_str())
    }

    fn contains_tab(&self, tab_id: &str) -> bool {
        self.tabs.iter().any(|tab| tab.tab_id == tab_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct TabInfo {
    tab_id: String,
    workspace_id: String,
    focused: bool,
}

#[derive(Debug, Deserialize)]
struct TabListResponse {
    result: TabListResult,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TabListResult {
    TabList { tabs: Vec<TabInfo> },
}

fn parse_tab_list_response(stdout: &[u8]) -> Result<TabSnapshot, PluginError> {
    let value: Value = serde_json::from_slice(stdout).map_err(|error| {
        PluginError::new(format!(
            "failed to parse HERDR_BIN_PATH tab list JSON: {error}"
        ))
    })?;

    if let Some(error) = herdr_error_message(&value) {
        return Err(PluginError::new(format!(
            "HERDR_BIN_PATH tab list returned an error: {error}"
        )));
    }

    let response: TabListResponse = serde_json::from_value(value).map_err(|error| {
        PluginError::new(format!(
            "HERDR_BIN_PATH tab list returned an unexpected response: {error}"
        ))
    })?;

    match response.result {
        TabListResult::TabList { tabs } => Ok(TabSnapshot { tabs }),
    }
}

fn herdr_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    let code = error.get("code").and_then(Value::as_str);
    let message = error.get("message").and_then(Value::as_str);

    match (code, message) {
        (Some(code), Some(message)) => Some(format!("{code}: {message}")),
        (Some(code), None) => Some(code.to_string()),
        (None, Some(message)) => Some(message.to_string()),
        (None, None) => Some(error.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Memory {
    Empty,
    Current {
        current: String,
        last: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct PersistedState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current_tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_tab_id: Option<String>,
    #[serde(default)]
    updated_unix_ms: u64,
}

impl PersistedState {
    fn from_memory(memory: &Memory, updated_unix_ms: u64) -> Self {
        match memory {
            Memory::Empty => Self {
                current_tab_id: None,
                last_tab_id: None,
                updated_unix_ms,
            },
            Memory::Current { current, last } => Self {
                current_tab_id: Some(current.clone()),
                last_tab_id: last.clone(),
                updated_unix_ms,
            },
        }
    }

    fn into_memory(self) -> LoadedMemory {
        let original_current = self.current_tab_id;
        let original_last = self.last_tab_id;
        let current = original_current.clone().filter(|id| !id.is_empty());
        let last = original_last.clone().filter(|id| !id.is_empty());

        match current {
            Some(current) => {
                let last = last.filter(|last| last != &current);
                let repaired =
                    original_current.as_deref() != Some(current.as_str()) || original_last != last;
                LoadedMemory {
                    memory: Memory::Current { current, last },
                    needs_repair: repaired,
                }
            }
            None => LoadedMemory {
                memory: Memory::Empty,
                needs_repair: original_current.is_some() || original_last.is_some(),
            },
        }
    }
}

struct LoadedMemory {
    memory: Memory,
    needs_repair: bool,
}

struct StateStore {
    state_dir: PathBuf,
}

impl StateStore {
    fn new(state_dir: &Path) -> Self {
        Self {
            state_dir: state_dir.to_path_buf(),
        }
    }

    fn update<T>(&self, change: impl FnOnce(Memory) -> (Memory, T)) -> Result<T, PluginError> {
        fs::create_dir_all(&self.state_dir).map_err(|error| {
            PluginError::new(format!(
                "failed to create plugin state directory {}: {error}",
                self.state_dir.display()
            ))
        })?;

        let _lock = StateLock::acquire(&self.state_dir.join(LOCK_FILE_NAME))?;
        let loaded = read_memory(&self.state_dir.join(STATE_FILE_NAME))?;
        let previous = loaded.memory.clone();
        let (next, result) = change(loaded.memory);

        if loaded.needs_repair || next != previous {
            write_memory(&self.state_dir.join(STATE_FILE_NAME), &next)?;
        }

        Ok(result)
    }
}

struct StateLock {
    file: File,
}

impl StateLock {
    fn acquire(path: &Path) -> Result<Self, PluginError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| {
                PluginError::new(format!(
                    "failed to open plugin state lock {}: {error}",
                    path.display()
                ))
            })?;

        file.lock_exclusive().map_err(|error| {
            PluginError::new(format!(
                "failed to lock plugin state {}: {error}",
                path.display()
            ))
        })?;

        Ok(Self { file })
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn read_memory(path: &Path) -> Result<LoadedMemory, PluginError> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LoadedMemory {
                memory: Memory::Empty,
                needs_repair: false,
            });
        }
        Err(error) => {
            return Err(PluginError::new(format!(
                "failed to read plugin state {}: {error}",
                path.display()
            )));
        }
    };

    match serde_json::from_str::<PersistedState>(&contents) {
        Ok(state) => Ok(state.into_memory()),
        Err(_) => Ok(LoadedMemory {
            memory: Memory::Empty,
            needs_repair: true,
        }),
    }
}

fn write_memory(path: &Path, memory: &Memory) -> Result<(), PluginError> {
    let parent = path.parent().ok_or_else(|| {
        PluginError::new(format!(
            "plugin state path has no parent directory: {}",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        PluginError::new(format!(
            "failed to create plugin state directory {}: {error}",
            parent.display()
        ))
    })?;

    let temp_path = parent.join(format!(
        ".{STATE_FILE_NAME}.tmp.{}.{}",
        std::process::id(),
        current_unix_ms()
    ));
    let mut temp_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .map_err(|error| {
            PluginError::new(format!(
                "failed to create temporary plugin state {}: {error}",
                temp_path.display()
            ))
        })?;
    let state = PersistedState::from_memory(memory, current_unix_ms());
    serde_json::to_writer_pretty(&mut temp_file, &state).map_err(|error| {
        PluginError::new(format!(
            "failed to serialize plugin state {}: {error}",
            temp_path.display()
        ))
    })?;
    temp_file.write_all(b"\n").map_err(|error| {
        PluginError::new(format!(
            "failed to write plugin state {}: {error}",
            temp_path.display()
        ))
    })?;
    temp_file.sync_all().map_err(|error| {
        PluginError::new(format!(
            "failed to sync plugin state {}: {error}",
            temp_path.display()
        ))
    })?;
    drop(temp_file);

    fs::rename(&temp_path, path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        PluginError::new(format!(
            "failed to replace plugin state {}: {error}",
            path.display()
        ))
    })
}

fn parse_event_tab_id(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    string_at(&value, &["data", "tab_id"]).or_else(|| string_at(&value, &["tab_id"]))
}

fn parse_context_tab_id(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    string_at(&value, &["tab_id"])
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PluginError {
    message: String,
}

impl PluginError {
    fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PluginError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeHerdr {
        snapshot: TabSnapshot,
        focused: RefCell<Vec<String>>,
    }

    impl FakeHerdr {
        fn new(tabs: Vec<(&str, bool)>) -> Self {
            Self {
                snapshot: snapshot(tabs),
                focused: RefCell::new(Vec::new()),
            }
        }
    }

    impl Herdr for FakeHerdr {
        fn tab_list(&self) -> Result<TabSnapshot, PluginError> {
            Ok(self.snapshot.clone())
        }

        fn focus_tab(&self, tab_id: &str) -> Result<(), PluginError> {
            self.focused.borrow_mut().push(tab_id.to_string());
            Ok(())
        }
    }

    fn runtime(state_dir: PathBuf) -> RuntimeEnv {
        RuntimeEnv {
            state_dir,
            event_json: None,
            context_json: None,
        }
    }

    fn snapshot(tabs: Vec<(&str, bool)>) -> TabSnapshot {
        TabSnapshot {
            tabs: tabs
                .into_iter()
                .map(|(tab_id, focused)| TabInfo {
                    tab_id: tab_id.to_string(),
                    workspace_id: tab_id.to_string(),
                    focused,
                })
                .collect(),
        }
    }

    fn temp_state_dir(label: &str) -> PathBuf {
        let path = env::temp_dir().join(format!(
            "herdr-last-tab-{label}-{}-{}",
            std::process::id(),
            current_unix_ms()
        ));
        fs::create_dir_all(&path).expect("temp state directory should be created");
        path
    }

    #[test]
    fn parses_tab_list_response() {
        let response = br#"{
            "id":"cli:tab:list",
            "result":{
                "type":"tab_list",
                "tabs":[
                    {"tab_id":"tab-1","workspace_id":"w1","number":2,"label":"B","focused":true},
                    {"tab_id":"tab-0","workspace_id":"w1","number":1,"label":"A","focused":false}
                ]
            }
        }"#;

        let snapshot = parse_tab_list_response(response).expect("response should parse");

        assert_eq!(snapshot.focused_tab_id(), Some("tab-1"));
        assert!(snapshot.contains_tab("tab-0"));
        assert!(!snapshot.contains_tab("tab-2"));
    }

    #[test]
    fn focused_event_updates_current_and_last_tab_ids() {
        let state_dir = temp_state_dir("focused-transition");
        let mut runtime = runtime(state_dir.clone());

        runtime.event_json = Some(event_json("workspace-a"));
        run(
            Subcommand::Focused,
            &runtime,
            &FakeHerdr::new(vec![("workspace-a", true), ("workspace-b", false)]),
        )
        .expect("first focused event should succeed");

        runtime.event_json = Some(event_json("workspace-b"));
        run(
            Subcommand::Focused,
            &runtime,
            &FakeHerdr::new(vec![("workspace-b", true), ("workspace-a", false)]),
        )
        .expect("second focused event should succeed");

        let loaded = read_memory(&state_dir.join(STATE_FILE_NAME)).expect("state should load");
        assert_eq!(
            loaded.memory,
            Memory::Current {
                current: "workspace-b".to_string(),
                last: Some("workspace-a".to_string())
            }
        );

        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn stale_focused_event_is_ignored() {
        let state_dir = temp_state_dir("stale-focused");
        write_memory(
            &state_dir.join(STATE_FILE_NAME),
            &Memory::Current {
                current: "workspace-a".to_string(),
                last: None,
            },
        )
        .expect("state should be written");
        let mut runtime = runtime(state_dir.clone());
        runtime.event_json = Some(event_json("workspace-b"));

        run(
            Subcommand::Focused,
            &runtime,
            &FakeHerdr::new(vec![("workspace-a", true), ("workspace-b", false)]),
        )
        .expect("stale event should be a clean no-op");

        let loaded = read_memory(&state_dir.join(STATE_FILE_NAME)).expect("state should load");
        assert_eq!(
            loaded.memory,
            Memory::Current {
                current: "workspace-a".to_string(),
                last: None,
            }
        );

        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn toggle_focuses_remembered_tab_by_id() {
        let state_dir = temp_state_dir("toggle");
        write_memory(
            &state_dir.join(STATE_FILE_NAME),
            &Memory::Current {
                current: "workspace-b".to_string(),
                last: Some("workspace-a".to_string()),
            },
        )
        .expect("state should be written");
        let runtime = runtime(state_dir.clone());
        let herdr = FakeHerdr::new(vec![("workspace-b", true), ("workspace-a", false)]);

        run(Subcommand::Toggle, &runtime, &herdr).expect("toggle should succeed");

        assert_eq!(herdr.focused.into_inner(), vec!["workspace-a".to_string()]);

        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn toggle_seeds_empty_state_without_focusing() {
        let state_dir = temp_state_dir("toggle-seed");
        let runtime = runtime(state_dir.clone());
        let herdr = FakeHerdr::new(vec![("workspace-a", true)]);

        run(Subcommand::Toggle, &runtime, &herdr).expect("toggle should seed state");

        assert!(herdr.focused.into_inner().is_empty());
        let loaded = read_memory(&state_dir.join(STATE_FILE_NAME)).expect("state should load");
        assert_eq!(
            loaded.memory,
            Memory::Current {
                current: "workspace-a".to_string(),
                last: None,
            }
        );

        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn toggle_clears_stale_last_tab_without_focusing() {
        let state_dir = temp_state_dir("toggle-stale-last");
        write_memory(
            &state_dir.join(STATE_FILE_NAME),
            &Memory::Current {
                current: "workspace-b".to_string(),
                last: Some("workspace-a".to_string()),
            },
        )
        .expect("state should be written");
        let runtime = runtime(state_dir.clone());
        let herdr = FakeHerdr::new(vec![("workspace-b", true)]);

        run(Subcommand::Toggle, &runtime, &herdr).expect("toggle should repair stale last");

        assert!(herdr.focused.into_inner().is_empty());
        let loaded = read_memory(&state_dir.join(STATE_FILE_NAME)).expect("state should load");
        assert_eq!(
            loaded.memory,
            Memory::Current {
                current: "workspace-b".to_string(),
                last: None,
            }
        );

        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn closed_event_clears_remembered_tab() {
        let state_dir = temp_state_dir("closed-last");
        write_memory(
            &state_dir.join(STATE_FILE_NAME),
            &Memory::Current {
                current: "workspace-b".to_string(),
                last: Some("workspace-a".to_string()),
            },
        )
        .expect("state should be written");
        let mut runtime = runtime(state_dir.clone());
        runtime.event_json = Some(event_json("workspace-a"));

        run(
            Subcommand::Closed,
            &runtime,
            &FakeHerdr::new(vec![("workspace-b", true)]),
        )
        .expect("closed event should succeed");

        let loaded = read_memory(&state_dir.join(STATE_FILE_NAME)).expect("state should load");
        assert_eq!(
            loaded.memory,
            Memory::Current {
                current: "workspace-b".to_string(),
                last: None,
            }
        );

        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn malformed_state_is_repaired() {
        let state_dir = temp_state_dir("malformed");
        fs::write(state_dir.join(STATE_FILE_NAME), "not json")
            .expect("malformed state should write");
        let runtime = runtime(state_dir.clone());
        let herdr = FakeHerdr::new(vec![("workspace-a", true)]);

        run(Subcommand::Toggle, &runtime, &herdr).expect("toggle should repair malformed state");

        let loaded = read_memory(&state_dir.join(STATE_FILE_NAME)).expect("state should load");
        assert_eq!(
            loaded.memory,
            Memory::Current {
                current: "workspace-a".to_string(),
                last: None,
            }
        );

        let _ = fs::remove_dir_all(state_dir);
    }

    fn event_json(tab_id: &str) -> String {
        format!(r#"{{"event":"tab_focused","data":{{"type":"tab_focused","tab_id":"{tab_id}"}}}}"#)
    }
}
