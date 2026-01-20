# AGENTS.md

## What this is
- `codex-launch` is a small Rust CLI that helps pick a folder and launch Codex CLI there, or pick a recent Codex session from `~/.codex/sessions` and run `codex resume <id>`.

## Commands
- Build: `cargo build`
- Run (interactive): `cargo run`
- Run (recent sessions): `cargo run -- --recent`
- Tests: `cargo test`

## Test
- Run: `cargo test`

## Build & setup
- Requires Rust toolchain: `rustup show`
- Install locally: `cargo install --path .`

## Lint & format
- Format: `cargo fmt`
- Lint: `cargo clippy -- -D warnings`

## Notes
- Config is stored at `~/.codex-launch/config.toml` and is created on first run.
- Repo discovery only scans direct children of each configured `projects.roots` (no deep recursion).
- Session listing reads only the first ~25 lines of each `rollout-*.jsonl` file to find `type:"session_meta"` (avoid scanning entire files).
- Repo targets are also inferred from recent session `cwd`s by default.
- Interactive project picker is a custom `crossterm` UI with keybinds (see `README.md`).

## Safety
- Never add or commit secrets (tokens, API keys, private URLs) to this repo.
