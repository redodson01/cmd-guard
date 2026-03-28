# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-03-28

### Added

- Core command parsing: splits compound commands on `&&`, `||`, `;`, `|`, `&` and
  evaluates each segment independently
- Shell wrapper unwrapping: `bash -c '...'` (and `sh`, `zsh`, `fish`, `dash`,
  `ksh`) with recursive nesting support and absolute path handling
- Git rules: deny `push --force`/`--force-with-lease`/`--mirror`, `add -A`,
  `stash clear`; ask for `push`, `reset --hard`, `clean -f`, `checkout .`,
  `checkout --force`, `restore .`, `branch -D`
- Git global flag stripping: `-C`, `-c`, `--git-dir`, `--work-tree` are removed
  before rule evaluation
- General rules: deny `rm -rf`, `find -delete`; ask for `rm -r`
- find `-exec`/`-execdir` child command extraction and evaluation
- Docker/Podman rules: deny `system prune -a`/`--all`; ask for `system prune`,
  any `prune` subcommand
- psql rules: deny `DROP DATABASE`/`DROP SCHEMA`; ask for `DROP TABLE`/`TRUNCATE`
- xargs child command extraction and evaluation
- Combined short flag detection (e.g., `-rf`, `-vf`, `-Av`)
- Depth-limited recursion (`MAX_SHELL_DEPTH = 16`) to prevent stack overflow
- `--setup` flag for self-contained post-install configuration (symlink +
  settings.json), removing the need for Python 3 or `configure-settings.sh`
- `--version` / `-V` flag
- Homebrew formula (`brew install redodson01/tap/cmd-guard`)
- Makefile with `install` and `uninstall` targets
- CI workflow testing against Rust stable and 1.85 (MSRV)
- Release workflow for cross-platform binaries with automatic Homebrew tap update
- Unit and integration test suites

[Unreleased]: https://github.com/redodson01/cmd-guard/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/redodson01/cmd-guard/releases/tag/v0.1.0
