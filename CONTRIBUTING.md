# Contributing to cmd-guard

## Development

```bash
# Run tests
cargo test

# Run clippy
cargo clippy -- -D warnings

# Format code
cargo fmt

# Build release binary
cargo build --release
```

Requires Rust 1.85+ (edition 2024).

## Adding rules

Rules live in `src/main.rs` in the `evaluate_git` and `evaluate_general`
functions. Each rule inspects the tokenized command and returns one of:

- `Some(("deny", reason))` -- block the command entirely (irreversible operations)
- `Some(("ask", reason))` -- prompt the user for confirmation (destructive but recoverable)
- `None` -- pass through silently (safe operations)

When adding a rule:

1. Add the detection logic in the appropriate `evaluate_*` function
2. Add unit tests in the `#[cfg(test)] mod tests` block using `process_segment_at_depth`
3. Add at least one integration test in `tests/integration.rs` if the rule
   covers a new command
4. Update the rules tables in `README.md`
5. Update `CHANGELOG.md` under `[Unreleased]`

## Pull requests

- Keep PRs focused on a single change
- Ensure `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt --check`
  all pass
- Use atomic, logically-ordered commits (this project does not squash merge)
- Follow the commit message style of the existing history
