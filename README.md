# codex-launch

Interactive launcher for Codex CLI that keeps a small registry of folders and helps you either:

- start a new Codex session in a chosen folder, or
- resume a recent session from `~/.codex/sessions`.

## Install (local)

```bash
cd /Users/ivan/Documents/Code/codex-launch-cli
cargo install --path .
```

## Quick start

Add a parent folder that contains multiple git repos (one-level scan):

```bash
codex-launch add-root ~/Documents/Code
```

Add an explicit folder target (git or non-git):

```bash
codex-launch add-path ~/.hammerspoon
```

Launch picker:

```bash
codex-launch
```

Resume a recent session (scoped to configured targets):

```bash
codex-launch --recent
```

Resume a recent session (all sessions):

```bash
codex-launch --recent --all-sessions
```

Non-interactive (no TTY): print recent sessions as TSV (`id<TAB>created_at<TAB>cwd`):

```bash
codex-launch --recent --no-ui --limit 20
```

## Config

Config is stored at `~/.codex-launch/config.toml` (created on first run).

Keys you’ll likely care about:

- `codex.bin`: the `codex` executable to run (default: `"codex"`)
- `codex.args`: default args passed to `codex`
- `projects.roots`: parent folders to scan one-level deep for repos
- `projects.paths`: explicit folder targets
- `sessions.codex_home`: where Codex keeps `sessions/` (default: `~/.codex`)
- `sessions.limit`: how many sessions to show (default: `15`)

## Notes

- `codex-launch` runs `codex` with `current_dir` set to the selected folder (or the session’s recorded `cwd` when resuming).
- Repo discovery only scans direct children of each configured `projects.roots`.
- Targets are also inferred from recent session `cwd`s by default by resolving the git repo root (`projects.from_sessions = true`).
