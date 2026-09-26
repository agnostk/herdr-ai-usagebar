# Troubleshooting

Start with the `status` action. Its output goes to herdr's plugin command log:

```sh
herdr plugin action invoke agnostk.ai-usagebar.status >/dev/null
herdr plugin log list --plugin agnostk.ai-usagebar --limit 1
```

It prints the plugin config path, which `ai-usagebar` it found, whether the
refresher is running (and its log path), the last refresh time and error, and
the state of each ai-usagebar entry. The refresher's own log is
`sessions/<id>/daemon.log` under the plugin state directory. Set
`HERDR_AI_USAGEBAR_DEBUG=1` in the environment you start herdr from for more
detail.

## Nothing shows in the sidebar

Check these in order:

1. **Is the layout installed?** herdr only renders tokens your layout names.
   Run `setup-sidebar`, or check that your `[ui.sidebar.agents]` /
   `[ui.sidebar.spaces]` rows contain `$ai_usage`. A `rows_by_agent` override
   replaces the rows entirely for that agent, so it needs the token too.
   `setup-sidebar` adds it to every override.
2. **Is the sidebar expanded?** Custom rows appear only in the expanded
   desktop sidebar, not the collapsed or mobile layouts.
3. **Is the refresher running?** `status` says so. If not, run the `refresh`
   action; after a fresh install it otherwise waits for the next agent,
   workspace switch, or herdr restart.
4. **Is the agent mapped?** Only agents in the
   [agent mapping](configuration.md#agent-mapping) get a row. Compare the id in
   `herdr agent list` with your `[agents]` table.
5. **Does ai-usagebar report that provider?** The entry has to appear in
   `ai-usagebar usage --json`. If it doesn't, enable the provider in
   ai-usagebar (`ai-usagebar detect`, or its `config.toml`).

## `⚠ ai-usagebar not found`

The refresher could not find the `ai-usagebar` binary. A herdr server started
outside your login shell may not have your full `PATH`, so the plugin also
searches `~/.cargo/bin`, `~/.local/bin`, `/opt/homebrew/bin`,
`/usr/local/bin` and the Nix profile directories. If yours lives elsewhere,
set it explicitly:

```toml
# $(herdr plugin config-dir agnostk.ai-usagebar)/config.toml
ai_usagebar = "/path/to/ai-usagebar"
```

If `ai_usagebar` is set but wrong, the plugin reports it as not found rather
than falling back to a search.

## `⚠ usage unavailable`

ai-usagebar ran but failed before producing any report. Common causes:
`no vendors enabled` in its config, a crash, or a run slower than
`fetch_timeout_secs`. The `status` action and the refresher log show the
redacted error. Run `ai-usagebar usage --json` yourself to see the full
message.

## `⚠ credentials error`, `⚠ rate limited`, …

ai-usagebar reached that provider but could not read it. The text is the
leading part of ai-usagebar's error for that entry. Fix it on the ai-usagebar
side (log in again, add the API key); the row recovers on the next refresh.
For rate limits, ai-usagebar backs off for five minutes by itself.

## `(stale)` after the numbers

The last `ai-usagebar usage` run failed, so the plugin keeps showing the
previous report rather than blanking the sidebar. It clears on the next
successful run.

## The numbers lag behind ai-usagebar-tui

The refresher runs ai-usagebar every `refresh_secs` (60 s by default), and
ai-usagebar itself caches each provider for 60 seconds. Run the `refresh`
action to fetch immediately.

## Two refreshers are running

That is expected with several herdr sessions (`herdr session list`): each
session has its own. Within a session, a lock guarantees a single refresher.

## `setup-sidebar` failed

- **`herdr rejected the updated config, nothing was changed`:** your
  `config.toml` already has an issue that `herdr config check` reports. Fix
  that first; the plugin never writes a config herdr would reject.
- **`herdr did not reload its config`:** the file was updated, but the reload
  call failed. Run `herdr server reload-config`, or run `setup-sidebar` again,
  which retries the reload.
- **`sidebar rows … is not an array`:** your layout uses an unusual shape.
  Add the row by hand; see [sidebar layout](configuration.md#sidebar-layout).

## Resetting

```sh
herdr plugin action invoke agnostk.ai-usagebar.stop
rm "$(herdr plugin config-dir agnostk.ai-usagebar)/config.toml"   # back to defaults
herdr plugin action invoke agnostk.ai-usagebar.refresh
```

To undo `setup-sidebar`, restore the `config.toml.bak-ai-usagebar-*` backup
next to your herdr config, or delete the `$ai_usage` rows, then run
`herdr server reload-config`.

## Still stuck?

[Open an issue](https://github.com/agnostk/herdr-ai-usagebar/issues/new/choose)
with your herdr version, the `status` output and the relevant lines from
`daemon.log`, redacted.
