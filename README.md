# cmd-guard

[![CI](https://github.com/redodson01/cmd-guard/actions/workflows/ci.yml/badge.svg)](https://github.com/redodson01/cmd-guard/actions/workflows/ci.yml)
[![Version](https://img.shields.io/github/v/release/redodson01/cmd-guard)](https://github.com/redodson01/cmd-guard/releases/latest)
[![Release Date](https://img.shields.io/github/release-date/redodson01/cmd-guard)](https://github.com/redodson01/cmd-guard/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A fast, compiled [PreToolUse hook](https://code.claude.com/docs/en/hooks) for
[Claude Code](https://claude.ai/code) that intercepts dangerous shell commands
before they execute. Written in Rust for near-zero cold start time.

Claude Code's built-in permission patterns (`Bash(git push:*)`) struggle with
compound commands — `cd repo && git push --force` bypasses the rule because the
full command string doesn't match the pattern. cmd-guard solves this by parsing
compound commands into segments and evaluating each one independently.

## How it works

cmd-guard reads the hook JSON from stdin, splits the command on shell operators
(`&&`, `||`, `;`, `|`, `&`), and evaluates every segment against a set of rules. The
strictest verdict wins: **deny** beats **ask** beats pass-through.

Three possible outcomes for each command:

| Decision | Effect | Example |
|----------|--------|---------|
| **deny** | Blocks the command entirely | `git push --force`, `git stash clear` |
| **ask** | Shows the permission prompt to the user | `git push origin main`, `git reset --hard` |
| *(none)* | Passes through silently | `git status`, `git diff`, `ls -la` |

## Rules

### Deny (irreversible / dangerous)

| Command | Reason |
|---------|--------|
| `git add -A` / `--all` | Stages everything, including files that shouldn't be committed |
| `git push --force` / `-f` / `--force-with-lease` / `--mirror` | Rewrites remote history |
| `git stash clear` | Permanently destroys all stash entries |
| `rm -rf` | Recursive forced deletion |
| `find ... -delete` | Permanent filesystem deletion |
| `docker system prune -a` / `--all` | Removes all unused images, containers, and networks (also matches `podman`) |
| `psql ... DROP DATABASE` / `DROP SCHEMA` | Data loss |

### Ask (destructive but recoverable)

| Command | Reason |
|---------|--------|
| `git add .` | Stages all changes in current directory |
| `git push --delete` / `-d` | Deletes a remote ref — confirm intent |
| `git push` (non-force) | Pushes to remote — confirm intent |
| `git reset --hard` | Discards uncommitted changes (recoverable via reflog) |
| `git clean -f` | Deletes untracked files permanently |
| `git checkout .` / `git restore .` | Discards uncommitted changes |
| `git checkout --force` / `-f` | Discards local modifications |
| `git branch -D` | Force-deletes a branch (recoverable via reflog) |
| `rm -r` (without `-f`) | Recursive deletion without force |
| `docker system prune` (without `-a`) | Removes dangling resources (also matches `podman`) |
| `docker ... prune` (any subcommand) | Removes unused resources (also matches `podman`) |
| `psql ... DROP TABLE` / `TRUNCATE` | Table-level data loss |

## Evasion resistance

cmd-guard handles several common ways commands can bypass naive pattern matching:

- **Compound commands**: `git status && git push --force` — each segment checked independently
- **Shell wrappers**: `bash -c 'git status && git push --force'` — unwrapped, inner compound commands split and evaluated independently (handles nested wrappers recursively, including absolute paths like `/bin/bash -c '...'`)
- **Git global flags**: `git -C /tmp push --force` — flags like `-C`, `-c`, `--git-dir`, `--work-tree` are stripped
- **xargs**: `echo main | xargs git push --force` — child command extracted and evaluated
- **find -exec**: `find . -exec rm -rf {} \;` — child command extracted from `-exec`/`-execdir` and evaluated against all rules
- **Quoting**: handles single and double quoted strings correctly

## Limitations

cmd-guard is a **productivity guardrail**, not a security boundary. It catches
the commands Claude Code is likely to generate, but it does not attempt to be a
comprehensive shell parser. Known gaps:

- **Backslash escapes**: `git\ push\ --force` is not parsed
- **Subshell substitution**: `$(git push --force)` and backtick substitution are
  not detected
- **Command prefixes**: `sudo git push --force`, `env git push --force`,
  `FOO=bar git push --force`, etc. bypass detection because the first token is
  the prefix (or assignment), not the command
- **psql detection**: catches destructive SQL keywords (`DROP`, `TRUNCATE`) in
  command-line arguments only; interactive sessions and file-based execution
  (`-f`) are not inspected

## Install

### Homebrew

```bash
brew install redodson01/tap/cmd-guard
cmd-guard --setup
```

### From source

Requires [Rust](https://rustup.rs/) 1.85+ (edition 2024).

```bash
cargo install --git https://github.com/redodson01/cmd-guard
cmd-guard --setup
```

Or clone and use make:

```bash
make install
```

### What `--setup` does

`cmd-guard --setup` is idempotent and handles all post-install configuration:

1. Symlinks the binary to `~/.claude/hooks/cmd-guard`
2. Adds the PreToolUse hook entry to `~/.claude/settings.json`

Re-run it after upgrading to update the symlink.

### Uninstall

```bash
make uninstall
```

Removes the binary and symlink. The hook entry in `settings.json` is left in
place — remove it manually if desired.

## Development

```bash
# Run tests
cargo test

# Lint and format (enforced by CI)
cargo clippy -- -D warnings
cargo fmt --check

# Build release binary
cargo build --release
```

## Credits

Inspired by the community of Claude Code users building hook-based permission
systems to work around limitations in the built-in pattern matching — see the
discussion on [anthropics/claude-code#16561](https://github.com/anthropics/claude-code/issues/16561)
and [anthropics/claude-code#30519](https://github.com/anthropics/claude-code/issues/30519).
