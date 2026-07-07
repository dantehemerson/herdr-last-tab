# Last Workspace Plugin PRD

Status: Draft
Date: 2026-06-22
Source branch: `last_workspace`
Closed PR: `ogulcancelik/herdr#708`

## Summary

Build a Herdr plugin that adds a "last workspace" action. The action toggles focus between the current workspace and the previously focused workspace, using stable workspace IDs instead of sidebar positions.

Users bind the action through `[[keys.command]]` with `type = "plugin_action"`. The plugin tracks focus changes with `workspace.focused` events, keeps its state under `HERDR_PLUGIN_STATE_DIR`, and calls back into Herdr with `HERDR_BIN_PATH workspace focus <workspace_id>`.

This gives users the branch behavior locally without adding a new native key field to Herdr core.

## Background

The branch added native support for:

- `keys.last_workspace` in config.
- `NavigateAction::LastWorkspace` in input handling.
- `AppState.last_workspace_id` as pure app state.
- Tests for toggling, workspace reorder, closed targets, and tab-driven workspace changes.

The PR was closed by the contributor-process bot, not rejected on product or technical review. Since Herdr's plugin API already exposes plugin actions, keybindings, event hooks, plugin state directories, and CLI access, this feature is a good plugin candidate.

## Problem

Users working across many workspaces often need to bounce between two active contexts. `previous_workspace` and `next_workspace` move through the visible order, but they do not capture recent focus history. Indexed workspace bindings work only when the target has a stable position.

The desired behavior is the tmux-style back-and-forth jump: if the user moves from workspace A to workspace B, the action should return to A. Pressing it again should return to B.

## Goals

- Provide a plugin action that toggles to the previously focused workspace.
- Track workspaces by Herdr workspace ID, not workspace number or sidebar index.
- Let users bind the action with normal plugin keybind syntax.
- No-op cleanly when no history exists.
- Clear stale history when the remembered workspace is closed.
- Keep all plugin state in `HERDR_PLUGIN_STATE_DIR`.
- Avoid changes to Herdr core for the first version.

## Non-Goals

- Add `keys.last_workspace` back to core.
- Add native UI beyond the existing keybind help entry for custom commands.
- Guarantee perfect history before the plugin has observed any focus event.
- Guarantee exact ordering for rapid focus changes if Herdr does not expose event sequence numbers.
- Replace `last_pane`; this plugin is workspace-only.

## User Experience

The user installs or links the plugin, then adds a keybind:

```toml
[[keys.command]]
key = "prefix+tab"
type = "plugin_action"
command = "dantehemerson.last-workspace.toggle"
description = "last workspace"
```

Expected behavior:

- With no recorded previous workspace, pressing the key does nothing and does not show an error toast.
- After focusing workspace A, then workspace B, pressing the key focuses A.
- Pressing the key again focuses B.
- Reordering workspaces does not affect the target.
- Closing the remembered workspace clears the target.

## Plugin API Fit

Existing Herdr APIs that support the plugin:

- Manifest actions: `[[actions]]` can declare `toggle`.
- Event hooks: `[[events]]` can listen to `workspace.focused` and `workspace.closed`.
- Plugin keybinds: `[[keys.command]]` with `type = "plugin_action"` invokes the action.
- Runtime context: `HERDR_PLUGIN_CONTEXT_JSON` and `HERDR_WORKSPACE_ID` expose the active workspace for actions.
- Durable state: `HERDR_PLUGIN_STATE_DIR` is owned by the plugin.
- CLI access: `HERDR_BIN_PATH workspace list` and `HERDR_BIN_PATH workspace focus <id>` provide read and write access.

Current API limits:

- `workspace.focused` includes only the newly focused workspace ID. It does not include `previous_workspace_id`.
- There is no plugin startup hook, so the plugin cannot know the active workspace before the first observed event or action.
- Event hooks run as separate external commands. They can finish out of order during rapid focus changes.

## Manifest Shape

```toml
id = "dantehemerson.last-workspace"
name = "Last Workspace"
version = "0.1.0"
min_herdr_version = "0.7.0"
description = "Toggle between the current and previously focused Herdr workspace."

[[actions]]
id = "toggle"
title = "Last workspace"
contexts = ["global", "workspace"]
command = ["./bin/last-workspace", "toggle"]

[[events]]
on = "workspace.focused"
command = ["./bin/last-workspace", "focused"]

[[events]]
on = "workspace.closed"
command = ["./bin/last-workspace", "closed"]
```

The exact command path can change with the implementation language. The entrypoint should still expose the same three subcommands: `toggle`, `focused`, and `closed`.

## State Model

State file: `$HERDR_PLUGIN_STATE_DIR/state.json`

```json
{
  "current_workspace_id": "workspace-1",
  "last_workspace_id": "workspace-0",
  "updated_unix_ms": 1782090000000
}
```

Field rules:

- `current_workspace_id` is the latest focused workspace the plugin accepts as current.
- `last_workspace_id` is the previous accepted current workspace.
- Missing or malformed state is treated as empty state.
- Writes use atomic replace.
- Updates take a short lock so action and hook commands do not corrupt the file.

## Event Handling

### `workspace.focused`

Inputs:

- `HERDR_PLUGIN_EVENT_JSON`, containing the event workspace ID.
- Current Herdr state from `HERDR_BIN_PATH workspace list`.

Algorithm:

1. Parse the event workspace ID.
2. Query `workspace list` and find the currently focused workspace.
3. If the event workspace ID is not the currently focused workspace, ignore the event as stale.
4. Lock state.
5. If `current_workspace_id` exists and differs from the event workspace ID, set `last_workspace_id` to the old `current_workspace_id`.
6. Set `current_workspace_id` to the event workspace ID.
7. If `last_workspace_id` equals `current_workspace_id`, clear `last_workspace_id`.
8. Write state atomically and release the lock.

The stale-event check is important. It reduces the risk from out-of-order hook completion when the user switches workspaces quickly.

### `workspace.closed`

Inputs:

- `HERDR_PLUGIN_EVENT_JSON`, containing the closed workspace ID.
- Current Herdr state from `HERDR_BIN_PATH workspace list`.

Algorithm:

1. Parse the closed workspace ID.
2. Query `workspace list` and find the currently focused workspace, if any.
3. Lock state.
4. If `last_workspace_id` equals the closed workspace ID, clear it.
5. If `current_workspace_id` equals the closed workspace ID, replace it with the currently focused workspace ID from `workspace list`.
6. If there is no focused workspace, clear `current_workspace_id`.
7. Write state atomically and release the lock.

This event should never leave state pointing at a closed workspace.

## Toggle Action

Inputs:

- `HERDR_PLUGIN_CONTEXT_JSON`, usually containing the active workspace ID.
- Current Herdr state from `HERDR_BIN_PATH workspace list`.

Algorithm:

1. Query `workspace list`.
2. Find the currently focused workspace ID.
3. Lock state.
4. If state is empty, seed `current_workspace_id` from the currently focused workspace and exit successfully.
5. If `last_workspace_id` is missing, exit successfully.
6. If `last_workspace_id` is not present in `workspace list`, clear it and exit successfully.
7. If `last_workspace_id` equals the currently focused workspace ID, clear it and exit successfully.
8. Save any repair from the steps above and release the lock.
9. Run `HERDR_BIN_PATH workspace focus <last_workspace_id>`.

The focus command will emit a later `workspace.focused` event. That event updates `current_workspace_id` and flips `last_workspace_id` for the next toggle.

## Edge Cases

- First run: no history exists, so the action seeds current workspace and exits.
- First observed focus event: records current workspace but cannot infer the previous one if state was empty.
- Workspace reorder: safe because state stores IDs.
- Closed last workspace: `workspace.closed` clears it.
- Closed current workspace: state follows Herdr's new focused workspace if one exists.
- Duplicate focus event: ignored because the event ID matches `current_workspace_id`.
- Stale focus event: ignored when event ID no longer matches Herdr's current focused workspace.
- Action pressed twice quickly: best effort. Without event sequence numbers, the second press may run before the first focus event updates state.

## Implementation Notes

Preferred implementation is a small compiled entrypoint with no runtime JSON parsing dependencies. Rust is a good fit if the plugin is meant to be shared; a local prototype can use any scripting language available on the user's machine.

The entrypoint should:

- Parse `HERDR_PLUGIN_EVENT_JSON` and `HERDR_PLUGIN_CONTEXT_JSON` defensively.
- Use `HERDR_BIN_PATH` instead of assuming `herdr` is on `PATH`.
- Treat missing Herdr env vars as a clean failure with an actionable stderr message.
- Exit `0` for normal no-op cases so Herdr does not show a custom command failure toast.
- Exit non-zero only when the plugin itself cannot run or cannot talk to Herdr.

## Acceptance Criteria

- After A -> B, invoking `toggle` focuses A.
- Invoking `toggle` again focuses B.
- Reordering A and B before invoking `toggle` still focuses the remembered workspace.
- Closing the remembered workspace makes `toggle` a no-op and clears `last_workspace_id`.
- The plugin writes no files outside `HERDR_PLUGIN_STATE_DIR`.
- Malformed state files are repaired or replaced without crashing.
- No-history and stale-history cases do not show Herdr error toasts.
- Rapid focus changes do not leave state pointing at a workspace that is not focused and was not recently focused, as long as stale events can be detected through `workspace list`.

## Test Plan

Manual tests:

- Link the plugin locally.
- Bind the action through `[[keys.command]]`.
- Start Herdr with three workspaces: A, B, C.
- Focus A, then B. Invoke toggle and verify A is focused.
- Invoke toggle again and verify B is focused.
- Focus C, reorder workspaces, invoke toggle and verify B is focused by ID.
- Close B when it is remembered as last. Invoke toggle and verify no error toast.
- Trigger rapid A -> B -> C focus changes and verify stale events do not overwrite C as current.

Automated tests for the plugin entrypoint:

- State transition tests for focused events.
- Closed-workspace cleanup tests.
- Toggle target selection tests with mocked `workspace list` output.
- Malformed state file repair test.
- Lock contention test with concurrent focused events.

## Known Differences From Native Core Support

- The plugin cannot know the previous workspace before it has observed a focus event or action.
- Keybinding uses `[[keys.command]]`, not a native `[keys] last_workspace` field.
- Plugin action completion and focus-event state updates are asynchronous.
- Exact event ordering cannot be guaranteed without a Herdr event sequence number.

## Possible Core API Extension

If exact parity becomes important, the smallest helpful Herdr change is to include `previous_workspace_id` on `workspace.focused` events. An event sequence or generation number would let plugins ignore out-of-order events without querying `workspace list`.

A startup or session-ready plugin hook would also let the plugin seed `current_workspace_id` before the first focus change, but `previous_workspace_id` is the more direct fix.

