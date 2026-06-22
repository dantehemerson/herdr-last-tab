# Herdr Last Workspace

Herdr plugin that adds one action: `third774.last-workspace.toggle`.

Use it to jump back to the workspace you were just using. Focus workspace A, move to workspace B, run the action, and Herdr focuses A. Run it again and Herdr focuses B.

The plugin tracks stable Herdr workspace IDs instead of workspace numbers, so it keeps working when workspace ordering changes.

## Install

Requires **Herdr >= 0.7.0**. The current manifest builds from source with Cargo, so you also need a local Rust toolchain.

```bash
herdr plugin install third774/herdr-last-workspace
```

Herdr clones the repo, runs the manifest build step, and registers the plugin action. Manage it with:

```bash
herdr plugin list
herdr plugin action list --plugin third774.last-workspace
herdr plugin uninstall third774.last-workspace
```

## Binding A Key

Add a `[[keys.command]]` entry to your Herdr config, usually `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+tab"
type = "plugin_action"
command = "third774.last-workspace.toggle"
description = "last workspace"
```

Reload Herdr config after editing:

```bash
herdr server reload-config
```

Then press your Herdr prefix (default `ctrl+b`) followed by the bound key.

## Usage

The first run usually records the current workspace and does not move focus because there is no previous workspace yet.

After you have focused at least two workspaces, running `third774.last-workspace.toggle` focuses the previously focused workspace. The next `workspace.focused` event updates the history, so repeated toggles switch between the same two workspaces.

Normal no-op cases exit cleanly without showing Herdr error toasts:

- No previous workspace has been recorded yet.
- The previous workspace was closed.
- Herdr does not report a focused workspace.

## How It Works

The plugin listens for Herdr's `workspace.focused` and `workspace.closed` events.

On focus events, it stores the current and previous workspace IDs under Herdr's plugin state directory. On closed events, it removes a remembered workspace if that workspace was closed. When the toggle action runs, it calls back into Herdr with:

```bash
$HERDR_BIN_PATH workspace list
$HERDR_BIN_PATH workspace focus <workspace_id>
```

State is stored as `state.json` with a `state.lock` file under `HERDR_PLUGIN_STATE_DIR`.

## Local Development

```bash
cargo build --release
herdr plugin link /path/to/herdr-last-workspace
```

Run the checks used for development:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

## Troubleshooting

List registered actions:

```bash
herdr plugin action list --plugin third774.last-workspace
```

Check plugin logs:

```bash
herdr plugin log list --plugin third774.last-workspace
```

If the action is missing, reinstall or relink the plugin and confirm the build step completed successfully.
