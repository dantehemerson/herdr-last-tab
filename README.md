# Herdr Last Tab

Based on: [third774/herdr-last-workspace](https://github.com/third774/herdr-last-workspace)

Herdr plugin that adds one action: `dantehemerson.last-tab.toggle`.

Use it to jump back to the tab you were just using. Focus tab A, move to tab B, run the action, and Herdr focuses A. Run it again and Herdr focuses B.

The plugin tracks stable Herdr tab IDs instead of tab numbers, so it keeps working when tab ordering changes.

## Install

Requires **Herdr >= 0.7.0**. The current manifest builds from source with Cargo, so you also need a local Rust toolchain.

```bash
herdr plugin install dantehemerson/herdr-last-tab
```

Herdr clones the repo, runs the manifest build step, and registers the plugin action. Manage it with:

```bash
herdr plugin list
herdr plugin action list --plugin dantehemerson.last-tab
herdr plugin uninstall dantehemerson.last-tab
```

## Binding A Key

Add a `[[keys.command]]` entry to your Herdr config, usually `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+space"
type = "plugin_action"
command = "dantehemerson.last-tab.toggle"
description = "last tab"
```

Reload Herdr config after editing:

```bash
herdr server reload-config
```

Then press your Herdr prefix (default `ctrl+b`) followed by the bound key.

## Usage

The first run usually records the current tab and does not move focus because there is no previous tab yet.

After you have focused at least two tabs, running `dantehemerson.last-tab.toggle` focuses the previously focused tab. The next `tab.focused` event updates the history, so repeated toggles switch between the same two tabs.

Normal no-op cases exit cleanly without showing Herdr error toasts:

- No previous tab has been recorded yet.
- The previous tab was closed.
- Herdr does not report a focused tab.

## How It Works

The plugin listens for Herdr's `tab.focused` and `tab.closed` events.

On focus events, it stores the current and previous tab IDs under Herdr's plugin state directory. On closed events, it removes a remembered tab if that tab was closed. When the toggle action runs, it calls back into Herdr with:

```bash
$HERDR_BIN_PATH tab list
$HERDR_BIN_PATH tab focus <tab_id>
```

State is stored as `state.json` with a `state.lock` file under `HERDR_PLUGIN_STATE_DIR`.

## Local Development

```bash
cargo build --release
herdr plugin link /path/to/herdr-last-tab
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
herdr plugin action list --plugin dantehemerson.last-tab
```

Check plugin logs:

```bash
herdr plugin log list --plugin dantehemerson.last-tab
```

If the action is missing, reinstall or relink the plugin and confirm the build step completed successfully.
