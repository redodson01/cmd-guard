use serde::{Deserialize, Serialize};
use std::io::{self, Read};

/// Maximum nesting depth for shell wrapper unwrapping to prevent stack
/// overflow from pathological input like `bash -c "bash -c '...'"`.
const MAX_SHELL_DEPTH: usize = 16;

// --- Input/Output types for Claude Code hooks ---

#[derive(Deserialize)]
struct HookInput {
    tool_input: ToolInput,
}

#[derive(Deserialize)]
struct ToolInput {
    command: String,
}

#[derive(Serialize)]
struct HookOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
}

#[derive(Serialize)]
struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: String,
    #[serde(rename = "permissionDecision")]
    permission_decision: String,
    #[serde(rename = "permissionDecisionReason")]
    permission_decision_reason: String,
}

// --- Decision output ---

fn decision(verdict: &str, reason: &str) -> ! {
    let output = HookOutput {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PreToolUse".to_string(),
            permission_decision: verdict.to_string(),
            permission_decision_reason: reason.to_string(),
        },
    };
    println!(
        "{}",
        serde_json::to_string(&output).expect("serialize hook output")
    );
    std::process::exit(0);
}

// --- Command parsing utilities ---

/// Split a command string into segments at shell operators (&&, ||, ;, |, &).
/// Each segment is evaluated independently.
fn split_segments(cmd: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = cmd.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < len {
        let c = chars[i];

        if c == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            current.push(c);
            i += 1;
            continue;
        }
        if c == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            current.push(c);
            i += 1;
            continue;
        }
        if in_single_quote || in_double_quote {
            current.push(c);
            i += 1;
            continue;
        }

        // Check for &&, ||
        if i + 1 < len && ((c == '&' && chars[i + 1] == '&') || (c == '|' && chars[i + 1] == '|')) {
            segments.push(current.trim().to_string());
            current = String::new();
            i += 2;
            continue;
        }
        // Check for ;, |, and & (background operator)
        if c == ';' || c == '|' || c == '&' {
            segments.push(current.trim().to_string());
            current = String::new();
            i += 1;
            continue;
        }

        current.push(c);
        i += 1;
    }

    let remainder = current.trim().to_string();
    if !remainder.is_empty() {
        segments.push(remainder);
    }

    segments
}

/// Tokenize a single command segment into words, handling basic quoting.
fn tokenize(segment: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = segment.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < len {
        let c = chars[i];

        if c == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            i += 1;
            continue;
        }
        if c == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            i += 1;
            continue;
        }

        if c.is_whitespace() && !in_single_quote && !in_double_quote {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            i += 1;
            continue;
        }

        current.push(c);
        i += 1;
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// Unwrap shell -c wrappers: `bash -c "git push --force"` -> `git push --force`
/// Returns the raw inner command string so the caller can re-split on shell
/// operators (the inner command may itself be compound).
///
/// Only `tokens[2]` is used — in real shell syntax, `bash -c 'cmd' arg0 arg1`
/// treats extra arguments as positional parameters (`$0`, `$1`), not as part
/// of the command string.
fn unwrap_shell(tokens: &[String]) -> Option<String> {
    if tokens.len() >= 3 && tokens[1] == "-c" {
        // Extract the binary name, handling absolute paths like /bin/bash
        let name = tokens[0].rsplit('/').next().unwrap_or(&tokens[0]);
        if matches!(name, "bash" | "sh" | "zsh" | "fish" | "dash" | "ksh") {
            return Some(tokens[2].clone());
        }
    }
    None
}

/// Strip git global flags that appear between `git` and the subcommand:
/// -C <path>, -c <key=value>, --git-dir=<path>, --work-tree=<path>
///
/// Only strips flags *before* the subcommand (the first non-flag token after
/// `git`). After the subcommand, flags like `-c` may have different meanings
/// (e.g., `git commit -c HEAD`) and must be preserved.
///
/// Respects the `--` end-of-options sentinel: tokens after `--` are passed
/// through verbatim so that path arguments like `-C` are not misinterpreted.
fn strip_git_flags(tokens: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut past_subcommand = false;

    while i < tokens.len() {
        let t = &tokens[i];

        // First token ("git") is always kept
        if i == 0 {
            result.push(t.clone());
            i += 1;
            continue;
        }

        // After the subcommand or --, pass through everything
        if past_subcommand {
            result.push(t.clone());
            i += 1;
            continue;
        }

        // -- before subcommand: keep it and pass through the rest
        if t == "--" {
            past_subcommand = true;
            result.push(t.clone());
            i += 1;
            continue;
        }

        // -C <path> or -c <key=value> (two tokens)
        if t == "-C" || t == "-c" {
            i += 2; // skip flag and its argument
            continue;
        }

        // --git-dir=... or --git-dir <path>
        if t.starts_with("--git-dir") {
            if t.contains('=') {
                i += 1;
            } else {
                i += 2;
            }
            continue;
        }

        // --work-tree=... or --work-tree <path>
        if t.starts_with("--work-tree") {
            if t.contains('=') {
                i += 1;
            } else {
                i += 2;
            }
            continue;
        }

        // Non-flag token = subcommand reached
        if !t.starts_with('-') {
            past_subcommand = true;
        }

        result.push(t.clone());
        i += 1;
    }

    result
}

/// Skip docker/podman global flags that precede the subcommand.
/// Analogous to `strip_git_flags` — prevents evasion via
/// `docker -H tcp://host system prune -a` or `docker --debug image prune`.
fn strip_docker_flags(tokens: &[String]) -> Vec<String> {
    if tokens.is_empty() {
        return tokens.to_vec();
    }
    let mut result = vec![tokens[0].clone()]; // keep "docker"/"podman"
    let mut i = 1;
    // Global flags that consume the next token as their argument
    let arg_flags = [
        "-H",
        "--host",
        "--config",
        "-c",
        "--context",
        "-l",
        "--log-level",
        "--tlscacert",
        "--tlscert",
        "--tlskey",
    ];

    // Skip global flags before the first subcommand
    while i < tokens.len() {
        let t = &tokens[i];

        // Non-flag token = subcommand reached; stop stripping
        if !t.starts_with('-') {
            break;
        }

        // --flag=value form (single token)
        if t.contains('=') {
            i += 1;
            continue;
        }

        // Known arg flags: skip flag and its argument
        if arg_flags.contains(&t.as_str()) {
            i += 2;
            continue;
        }

        // Boolean flag (-D, --debug, --tls, --tlsverify, etc.)
        i += 1;
    }

    // Keep the subcommand and everything after it
    result.extend(tokens.get(i..).unwrap_or_default().iter().cloned());
    result
}

/// Check if any token matches a value exactly.
fn has_token(tokens: &[String], value: &str) -> bool {
    tokens.iter().any(|t| t == value)
}

/// Check if any short-flag token (starts with `-` but not `--`) contains the given character.
/// Catches combined flags like `-rf`, `-vf`, `-Av`, etc.
/// Stops at the `--` end-of-options sentinel so that positional arguments
/// like filenames starting with `-` are not mistaken for flags.
fn has_short_flag_char(tokens: &[String], ch: char) -> bool {
    tokens
        .iter()
        .take_while(|t| t.as_str() != "--")
        .any(|t| t.starts_with('-') && !t.starts_with("--") && t.contains(ch))
}

/// Check if any token matches a force flag (-f, -vf, --force, etc.).
fn has_force_flag(tokens: &[String]) -> bool {
    has_token(tokens, "--force") || has_short_flag_char(tokens, 'f')
}

// --- Rule evaluation ---

/// Evaluate a normalized set of tokens against git rules.
/// Returns Some(("deny"|"ask", reason)) or None to pass through.
fn evaluate_git(tokens: &[String]) -> Option<(&'static str, String)> {
    if tokens.is_empty() || tokens[0] != "git" {
        return None;
    }

    // Find the subcommand (first token after "git" that doesn't start with -)
    let sub = tokens.iter().skip(1).find(|t| !t.starts_with('-'));
    let sub = match sub {
        Some(s) => s.as_str(),
        None => return None,
    };

    match sub {
        // --- Deny rules (irreversible) ---
        "add" => {
            if has_token(tokens, "--all") || has_short_flag_char(tokens, 'A') {
                return Some(("deny", "git add -A/--all blocked by policy".into()));
            }
            if has_token(tokens, ".") || has_token(tokens, "./") {
                return Some((
                    "ask",
                    "git add . — stages all changes in current directory".into(),
                ));
            }
        }
        "push" => {
            if has_force_flag(tokens)
                || tokens.iter().any(|t| t.starts_with("--force-with-lease"))
                || has_token(tokens, "--mirror")
            {
                return Some(("deny", "Force push blocked — remote history rewrite".into()));
            }
            if has_token(tokens, "--delete") || has_short_flag_char(tokens, 'd') {
                return Some(("ask", "Confirm: delete remote ref?".into()));
            }
            return Some(("ask", "Confirm: push to remote?".into()));
        }
        "stash" => {
            if has_token(tokens, "clear") {
                return Some(("deny", "git stash clear blocked — permanent loss".into()));
            }
        }

        // --- Ask rules (destructive but recoverable) ---
        "reset" => {
            if has_token(tokens, "--hard") {
                return Some(("ask", "git reset --hard — recoverable via reflog".into()));
            }
        }
        "clean" => {
            if has_force_flag(tokens) {
                return Some((
                    "ask",
                    "git clean -f — deletes untracked files permanently".into(),
                ));
            }
        }
        "checkout" => {
            if has_token(tokens, ".") || has_token(tokens, "./") {
                return Some((
                    "ask",
                    "git checkout . — discards uncommitted changes".into(),
                ));
            }
            if has_force_flag(tokens) {
                return Some((
                    "ask",
                    "git checkout --force — discards local modifications".into(),
                ));
            }
        }
        "restore" => {
            if has_token(tokens, ".") || has_token(tokens, "./") {
                // git restore --staged . only unstages — non-destructive
                let staged = has_token(tokens, "--staged");
                let worktree = has_token(tokens, "--worktree");
                if !staged || worktree {
                    return Some(("ask", "git restore . — discards uncommitted changes".into()));
                }
            }
        }
        "branch" => {
            if has_short_flag_char(tokens, 'D') {
                return Some(("ask", "git branch -D — recoverable via reflog".into()));
            }
        }

        _ => {}
    }

    None
}

/// Evaluate tokens against non-git rules (find, psql, docker/podman).
///
/// Note: psql detection is best-effort — it joins all tokens and searches
/// for SQL keywords as substrings, so `-c "DROP TABLE ..."` is caught.
/// It does not detect destructive statements in interactive sessions.
fn evaluate_general(tokens: &[String]) -> Option<(&'static str, String)> {
    if tokens.is_empty() {
        return None;
    }

    let cmd = tokens[0].as_str();

    match cmd {
        "rm" => {
            let has_recursive = has_token(tokens, "--recursive")
                || has_short_flag_char(tokens, 'r')
                || has_short_flag_char(tokens, 'R');
            let has_force = has_force_flag(tokens);
            if has_recursive && has_force {
                return Some(("deny", "rm -rf blocked — recursive forced deletion".into()));
            }
            if has_recursive {
                return Some(("ask", "rm -r — confirm recursive deletion?".into()));
            }
        }
        "find" => {
            if has_token(tokens, "-delete") {
                return Some(("deny", "find -delete blocked — permanent deletion".into()));
            }
            // Check all -exec/-execdir clauses for destructive child commands
            let mut worst: Option<(&'static str, String)> = None;
            for (i, t) in tokens.iter().enumerate() {
                if t != "-exec" && t != "-execdir" {
                    continue;
                }
                let child: Vec<String> = tokens[i + 1..]
                    .iter()
                    .take_while(|t| t.as_str() != ";" && t.as_str() != r"\;" && t.as_str() != "+")
                    .filter(|t| t.as_str() != "{}")
                    .cloned()
                    .collect();
                if child.is_empty() {
                    continue;
                }
                // Unwrap shell wrappers (e.g., find -exec bash -c 'rm -rf' \;)
                if let Some(inner) = unwrap_shell(&child) {
                    if let Some((verdict, reason)) = evaluate_command(&inner) {
                        match verdict {
                            "deny" => return Some(("deny", reason)),
                            "ask" => worst = Some((verdict, reason)),
                            _ => {}
                        }
                    }
                    continue;
                }
                let child = if child[0] == "git" {
                    strip_git_flags(&child)
                } else {
                    child
                };
                if let Some((verdict, reason)) =
                    evaluate_git(&child).or_else(|| evaluate_general(&child))
                {
                    match verdict {
                        "deny" => return Some(("deny", reason)),
                        "ask" => worst = Some((verdict, reason)),
                        _ => {}
                    }
                }
            }
            if worst.is_some() {
                return worst;
            }
        }
        "psql" => {
            let words: Vec<String> = tokens[1..]
                .iter()
                .flat_map(|t| t.split_whitespace())
                .map(str::to_uppercase)
                .collect();
            let has_drop =
                |target: &str| words.windows(2).any(|w| w[0] == "DROP" && w[1] == target);
            if has_drop("DATABASE") || has_drop("SCHEMA") {
                return Some(("deny", "DROP DATABASE/SCHEMA blocked — data loss".into()));
            }
            if has_drop("TABLE") || words.iter().any(|w| w == "TRUNCATE") {
                return Some(("ask", "DROP TABLE/TRUNCATE — confirm?".into()));
            }
        }
        "docker" | "podman" => {
            // Strip global flags so `docker -H host system prune` is still caught
            let tokens = strip_docker_flags(tokens);
            // Check subcommands positionally to avoid false-positives from
            // tokens like `docker build --tag prune .`
            if tokens.len() >= 3 && tokens[1] == "system" && tokens[2] == "prune" {
                if has_token(&tokens, "--all") || has_short_flag_char(&tokens, 'a') {
                    return Some((
                        "deny",
                        "system prune --all blocked — removes all unused resources".into(),
                    ));
                }
                return Some(("ask", "system prune — removes dangling resources".into()));
            }
            if tokens.len() >= 3
                && tokens[2] == "prune"
                && matches!(
                    tokens[1].as_str(),
                    "image" | "container" | "volume" | "network" | "builder" | "buildx"
                )
            {
                return Some(("ask", "prune — removes unused resources".into()));
            }
        }
        _ => {}
    }

    None
}

/// Evaluate xargs: extract the child command and evaluate it.
/// Skips xargs' own flags (before the child command), then passes the
/// entire child command — including its flags — to the rule evaluators.
fn evaluate_xargs(tokens: &[String]) -> Option<(&'static str, String)> {
    let xargs_pos = tokens.iter().position(|t| t == "xargs")?;
    let after_xargs = &tokens[xargs_pos + 1..];

    // Skip xargs' own flags to find where the child command starts.
    // Some flags take an argument (-I {}, -n 1, etc.) which must also be skipped.
    let xargs_arg_flags = ["-I", "-n", "-d", "-P", "-L", "-s", "-a", "-E"];
    let mut j = 0;
    while j < after_xargs.len() {
        let t = &after_xargs[j];
        if !t.starts_with('-') {
            break;
        }
        if xargs_arg_flags.contains(&t.as_str()) {
            j += 2; // skip flag and its argument
        } else {
            j += 1;
        }
    }
    let child: Vec<String> = after_xargs.get(j..).unwrap_or_default().to_vec();

    if child.is_empty() {
        return None;
    }

    // Strip git global flags so evasion resistance composes with xargs
    let child = if child.first().is_some_and(|t| t == "git") {
        strip_git_flags(&child)
    } else {
        child
    };

    evaluate_git(&child).or_else(|| evaluate_general(&child))
}

/// Process a single command segment: normalize and evaluate.
fn process_segment_at_depth(segment: &str, depth: usize) -> Option<(&'static str, String)> {
    let tokens = tokenize(segment);

    // Unwrap shell -c wrappers and re-evaluate the inner command, which
    // may itself be compound (e.g. `bash -c 'git status && git push --force'`).
    if let Some(inner) = unwrap_shell(&tokens) {
        return evaluate_command_at_depth(&inner, depth + 1);
    }

    if tokens.is_empty() {
        return None;
    }

    // Check for xargs patterns
    if let result @ Some(_) = evaluate_xargs(&tokens) {
        return result;
    }

    // If git command, strip global flags before evaluating
    let tokens = if tokens[0] == "git" {
        strip_git_flags(&tokens)
    } else {
        tokens
    };

    evaluate_git(&tokens).or_else(|| evaluate_general(&tokens))
}

/// Evaluate a full command string: split into segments, evaluate each,
/// and return the strictest verdict (deny > ask > pass-through).
fn evaluate_command(command: &str) -> Option<(&'static str, String)> {
    evaluate_command_at_depth(command, 0)
}

fn evaluate_command_at_depth(command: &str, depth: usize) -> Option<(&'static str, String)> {
    if depth > MAX_SHELL_DEPTH {
        return None;
    }
    let segments = split_segments(command);
    let mut worst: Option<(&'static str, String)> = None;

    for seg in &segments {
        if let Some((verdict, reason)) = process_segment_at_depth(seg, depth) {
            match verdict {
                "deny" => return Some(("deny", reason)),
                "ask" => worst = Some((verdict, reason)),
                _ => {}
            }
        }
    }

    worst
}

fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        // Can't read input — pass through silently
        std::process::exit(0);
    }

    let hook_input: HookInput = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => std::process::exit(0), // Not a command we can parse — pass through
    };

    if let Some((verdict, reason)) = evaluate_command(&hook_input.tool_input.command) {
        decision(verdict, &reason);
    }

    // No verdict — pass through (no output, exit 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_segments() {
        let segs = split_segments("git add . && git commit -m 'hello world'");
        assert_eq!(segs, vec!["git add .", "git commit -m 'hello world'"]);

        let segs = split_segments("git status; git push");
        assert_eq!(segs, vec!["git status", "git push"]);

        let segs = split_segments("echo foo | grep bar");
        assert_eq!(segs, vec!["echo foo", "grep bar"]);
    }

    #[test]
    fn test_split_segments_background_operator() {
        // Single & (background operator) should be treated as a separator
        let segs = split_segments("git status & git push --force");
        assert_eq!(segs, vec!["git status", "git push --force"]);

        // Make sure && still works
        let segs = split_segments("git status && git push --force");
        assert_eq!(segs, vec!["git status", "git push --force"]);
    }

    #[test]
    fn test_tokenize() {
        assert_eq!(
            tokenize("git commit -m 'hello world'"),
            vec!["git", "commit", "-m", "hello world"]
        );
        assert_eq!(
            tokenize(r#"git commit -m "hello world""#),
            vec!["git", "commit", "-m", "hello world"]
        );
    }

    #[test]
    fn test_strip_git_flags() {
        let tokens: Vec<String> = vec!["git", "-C", "/tmp", "reset", "--hard"]
            .into_iter()
            .map(String::from)
            .collect();
        let stripped = strip_git_flags(&tokens);
        assert_eq!(stripped, vec!["git", "reset", "--hard"]);

        let tokens: Vec<String> = vec!["git", "--git-dir=/foo", "push", "--force"]
            .into_iter()
            .map(String::from)
            .collect();
        let stripped = strip_git_flags(&tokens);
        assert_eq!(stripped, vec!["git", "push", "--force"]);

        // --work-tree with = form
        let tokens: Vec<String> = vec!["git", "--work-tree=/foo", "push", "--force"]
            .into_iter()
            .map(String::from)
            .collect();
        let stripped = strip_git_flags(&tokens);
        assert_eq!(stripped, vec!["git", "push", "--force"]);

        // --work-tree with space-separated argument
        let tokens: Vec<String> = vec!["git", "--work-tree", "/foo", "push", "--force"]
            .into_iter()
            .map(String::from)
            .collect();
        let stripped = strip_git_flags(&tokens);
        assert_eq!(stripped, vec!["git", "push", "--force"]);

        // -c after subcommand is preserved (subcommand-level flag, not global)
        let tokens: Vec<String> = vec!["git", "commit", "-c", "HEAD"]
            .into_iter()
            .map(String::from)
            .collect();
        let stripped = strip_git_flags(&tokens);
        assert_eq!(stripped, vec!["git", "commit", "-c", "HEAD"]);

        // -c before subcommand is still stripped (global flag)
        let tokens: Vec<String> = vec!["git", "-c", "push.default=current", "push", "--force"]
            .into_iter()
            .map(String::from)
            .collect();
        let stripped = strip_git_flags(&tokens);
        assert_eq!(stripped, vec!["git", "push", "--force"]);
    }

    #[test]
    fn test_deny_git_add_all() {
        assert!(matches!(
            process_segment_at_depth("git add -A", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("git add --all", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_ask_git_add_dot() {
        assert!(matches!(
            process_segment_at_depth("git add .", 0),
            Some(("ask", _))
        ));
        // ./ variant also caught
        assert!(matches!(
            process_segment_at_depth("git add ./", 0),
            Some(("ask", _))
        ));
        // Specific file path still passes through
        assert!(process_segment_at_depth("git add file.rs", 0).is_none());
        assert!(process_segment_at_depth("git add src/main.rs", 0).is_none());
    }

    #[test]
    fn test_deny_force_push() {
        assert!(matches!(
            process_segment_at_depth("git push --force", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("git push -f origin main", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_deny_force_push_with_lease() {
        assert!(matches!(
            process_segment_at_depth("git push --force-with-lease", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("git push --force-with-lease=origin/main", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_deny_push_mirror() {
        assert!(matches!(
            process_segment_at_depth("git push --mirror", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("git push --mirror origin", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_deny_stash_clear() {
        assert!(matches!(
            process_segment_at_depth("git stash clear", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_ask_push() {
        assert!(matches!(
            process_segment_at_depth("git push", 0),
            Some(("ask", _))
        ));
        assert!(matches!(
            process_segment_at_depth("git push origin main", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_ask_push_delete() {
        // --delete and -d should produce a specific "delete remote ref" message
        let result = process_segment_at_depth("git push --delete origin feature", 0);
        assert!(matches!(result, Some(("ask", ref r)) if r.contains("delete remote ref")));
        let result = process_segment_at_depth("git push -d origin feature", 0);
        assert!(matches!(result, Some(("ask", ref r)) if r.contains("delete remote ref")));
        // Plain push should NOT get the delete message
        let result = process_segment_at_depth("git push origin main", 0);
        assert!(matches!(result, Some(("ask", ref r)) if !r.contains("delete")));
    }

    #[test]
    fn test_ask_reset_hard() {
        assert!(matches!(
            process_segment_at_depth("git reset --hard HEAD~1", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_ask_clean() {
        assert!(matches!(
            process_segment_at_depth("git clean -f", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_ask_checkout_dot() {
        assert!(matches!(
            process_segment_at_depth("git checkout .", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_ask_checkout_force() {
        assert!(matches!(
            process_segment_at_depth("git checkout -f HEAD~1", 0),
            Some(("ask", _))
        ));
        assert!(matches!(
            process_segment_at_depth("git checkout --force main", 0),
            Some(("ask", _))
        ));
        // Safe checkout passes through
        assert!(process_segment_at_depth("git checkout main", 0).is_none());
    }

    #[test]
    fn test_ask_restore_dot() {
        assert!(matches!(
            process_segment_at_depth("git restore .", 0),
            Some(("ask", _))
        ));
        // --staged only unstages — non-destructive, should pass through
        assert!(process_segment_at_depth("git restore --staged .", 0).is_none());
        // --staged --worktree restores both — destructive
        assert!(matches!(
            process_segment_at_depth("git restore --staged --worktree .", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_ask_branch_delete() {
        assert!(matches!(
            process_segment_at_depth("git branch -D feature", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_bypass_via_c_flag() {
        assert!(matches!(
            process_segment_at_depth("git -C /tmp push --force", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("git -C /tmp reset --hard", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_bypass_via_c_config_flag() {
        assert!(matches!(
            process_segment_at_depth("git -c push.default=current push --force", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_shell_wrapper_unwrap() {
        assert!(matches!(
            process_segment_at_depth("bash -c 'git push --force'", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_shell_wrapper_compound() {
        // Compound command inside shell wrapper: inner && is re-split
        assert!(matches!(
            process_segment_at_depth("bash -c 'git status && git push --force'", 0),
            Some(("deny", _))
        ));
        // Ask verdict from inner compound
        assert!(matches!(
            process_segment_at_depth("sh -c 'git status && git push origin main'", 0),
            Some(("ask", _))
        ));
        // Safe inner compound passes through
        assert!(process_segment_at_depth("bash -c 'git status && git diff'", 0).is_none());
    }

    #[test]
    fn test_shell_wrapper_non_git() {
        assert!(matches!(
            process_segment_at_depth("bash -c 'rm -rf /'", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_shell_wrapper_absolute_path() {
        assert!(matches!(
            process_segment_at_depth("/bin/bash -c 'git push --force'", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("/usr/bin/sh -c 'rm -rf /'", 0),
            Some(("deny", _))
        ));
        assert!(!process_segment_at_depth("/bin/bash -c 'git status'", 0).is_some());
    }

    #[test]
    fn test_shell_wrapper_nested() {
        // Nested shell wrappers are handled recursively
        assert!(matches!(
            process_segment_at_depth(r#"bash -c "bash -c 'git push --force'""#, 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_deny_rm_rf() {
        assert!(matches!(
            process_segment_at_depth("rm -rf /tmp/foo", 0),
            Some(("deny", _))
        ));
        // Combined flags in different order
        assert!(matches!(
            process_segment_at_depth("rm -fr /tmp/foo", 0),
            Some(("deny", _))
        ));
        // Separate flags
        assert!(matches!(
            process_segment_at_depth("rm -r -f /tmp/foo", 0),
            Some(("deny", _))
        ));
        // Uppercase -R
        assert!(matches!(
            process_segment_at_depth("rm -Rf /tmp/foo", 0),
            Some(("deny", _))
        ));
        // Long form
        assert!(matches!(
            process_segment_at_depth("rm --recursive --force /tmp/foo", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_ask_rm_r() {
        assert!(matches!(
            process_segment_at_depth("rm -r /tmp/foo", 0),
            Some(("ask", _))
        ));
        assert!(matches!(
            process_segment_at_depth("rm -R /tmp/foo", 0),
            Some(("ask", _))
        ));
        assert!(matches!(
            process_segment_at_depth("rm --recursive /tmp/foo", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_rm_safe() {
        // Non-recursive rm passes through
        assert!(process_segment_at_depth("rm file.txt", 0).is_none());
        assert!(process_segment_at_depth("rm -f file.txt", 0).is_none());
    }

    #[test]
    fn test_find_delete() {
        assert!(matches!(
            process_segment_at_depth("find . -name '*.tmp' -delete", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_find_exec_dangerous() {
        // find -exec with destructive child commands
        assert!(matches!(
            process_segment_at_depth(r"find . -exec rm -rf {} \;", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("find . -exec rm -rf {} +", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("find . -execdir rm -rf {} +", 0),
            Some(("deny", _))
        ));
        // find -exec with git commands
        assert!(matches!(
            process_segment_at_depth(r"find . -name '*.git' -exec git push --force {} \;", 0),
            Some(("deny", _))
        ));
        // find -exec with ask-level child
        assert!(matches!(
            process_segment_at_depth("find . -exec rm -r {} +", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_find_multiple_exec() {
        // Second -exec with destructive command should be caught
        assert!(matches!(
            process_segment_at_depth(r"find . -exec echo {} \; -exec rm -rf {} \;", 0),
            Some(("deny", _))
        ));
        // First -exec safe, second -exec ask-level
        assert!(matches!(
            process_segment_at_depth(r"find . -exec echo {} \; -exec rm -r {} \;", 0),
            Some(("ask", _))
        ));
        // Both -exec safe
        assert!(
            process_segment_at_depth(r"find . -exec echo {} \; -exec chmod 644 {} \;", 0).is_none()
        );
    }

    #[test]
    fn test_find_exec_shell_wrapper() {
        // find -exec bash -c 'rm -rf' should be caught via shell unwrapping
        assert!(matches!(
            process_segment_at_depth(r#"find . -exec bash -c "rm -rf {}" \;"#, 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth(r"find . -exec sh -c 'git push --force' \;", 0),
            Some(("deny", _))
        ));
        // Safe shell wrapper should pass through
        assert!(process_segment_at_depth(r"find . -exec bash -c 'echo hello' \;", 0).is_none());
    }

    #[test]
    fn test_find_exec_safe() {
        assert!(process_segment_at_depth("find . -exec echo {} +", 0).is_none());
        assert!(process_segment_at_depth(r"find . -exec chmod 644 {} \;", 0).is_none());
        assert!(process_segment_at_depth(r"find . -execdir echo {} \;", 0).is_none());
    }

    #[test]
    fn test_safe_commands_pass() {
        assert!(process_segment_at_depth("git status", 0).is_none());
        assert!(process_segment_at_depth("git diff", 0).is_none());
        assert!(process_segment_at_depth("git log --oneline", 0).is_none());
        assert!(process_segment_at_depth("git add file.rs", 0).is_none());
        assert!(process_segment_at_depth("ls -la", 0).is_none());
    }

    #[test]
    fn test_docker_system_prune_all() {
        assert!(matches!(
            process_segment_at_depth("docker system prune -a", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("docker system prune --all", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_docker_system_prune() {
        assert!(matches!(
            process_segment_at_depth("docker system prune", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_docker_volume_prune() {
        assert!(matches!(
            process_segment_at_depth("docker volume prune", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_podman_system_prune_all() {
        assert!(matches!(
            process_segment_at_depth("podman system prune -a", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("podman system prune --all", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_podman_system_prune() {
        assert!(matches!(
            process_segment_at_depth("podman system prune", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_podman_volume_prune() {
        assert!(matches!(
            process_segment_at_depth("podman volume prune", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_docker_global_flags_bypass() {
        // Docker global flags should be stripped before subcommand detection
        assert!(matches!(
            process_segment_at_depth("docker --debug system prune -a", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("docker -H tcp://localhost:2375 system prune", 0),
            Some(("ask", _))
        ));
        assert!(matches!(
            process_segment_at_depth("docker -D image prune", 0),
            Some(("ask", _))
        ));
        // Podman too
        assert!(matches!(
            process_segment_at_depth("podman --log-level debug system prune --all", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_strip_docker_flags() {
        let tokens: Vec<String> = vec!["docker", "--debug", "system", "prune", "-a"]
            .into_iter()
            .map(String::from)
            .collect();
        let stripped = strip_docker_flags(&tokens);
        assert_eq!(stripped, vec!["docker", "system", "prune", "-a"]);

        let tokens: Vec<String> = vec!["docker", "-H", "tcp://host", "image", "prune"]
            .into_iter()
            .map(String::from)
            .collect();
        let stripped = strip_docker_flags(&tokens);
        assert_eq!(stripped, vec!["docker", "image", "prune"]);

        // Truncated: arg flag at end with no value
        let tokens: Vec<String> = vec!["docker", "-H"].into_iter().map(String::from).collect();
        let stripped = strip_docker_flags(&tokens);
        assert_eq!(stripped, vec!["docker"]);
    }

    #[test]
    fn test_psql_drop_database() {
        assert!(matches!(
            process_segment_at_depth("psql -h localhost -c DROP DATABASE mydb", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_psql_drop_schema() {
        assert!(matches!(
            process_segment_at_depth("psql -c DROP SCHEMA public", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_psql_drop_table() {
        assert!(matches!(
            process_segment_at_depth("psql -c DROP TABLE users", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_psql_truncate() {
        assert!(matches!(
            process_segment_at_depth("psql -c TRUNCATE users", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_psql_safe() {
        assert!(process_segment_at_depth("psql -h localhost mydb", 0).is_none());
        // Table/column names containing SQL keywords should not false-positive
        assert!(process_segment_at_depth("psql -c 'SELECT truncated_at FROM posts'", 0).is_none());
        assert!(process_segment_at_depth("psql -c 'SELECT * FROM drop_schema_logs'", 0).is_none());
    }

    #[test]
    fn test_xargs_git_push_force() {
        // After pipe splitting, xargs sees: "xargs git push --force"
        assert!(matches!(
            process_segment_at_depth("xargs git push --force", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_xargs_find_delete() {
        assert!(matches!(
            process_segment_at_depth("xargs find . -delete", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_xargs_safe() {
        assert!(process_segment_at_depth("xargs grep TODO", 0).is_none());
    }

    #[test]
    fn test_xargs_with_flags() {
        assert!(matches!(
            process_segment_at_depth("xargs -n 1 git push --force", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("xargs -I {} git push --force", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_xargs_with_git_global_flags() {
        assert!(matches!(
            process_segment_at_depth("xargs git -C /tmp push --force", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_docker_combined_short_flags() {
        assert!(matches!(
            process_segment_at_depth("docker system prune -af", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("podman system prune -fa", 0),
            Some(("deny", _))
        ));
    }

    #[test]
    fn test_pipe_xargs_end_to_end() {
        assert_eq!(
            evaluate_command("echo main | xargs git push --force")
                .unwrap()
                .0,
            "deny"
        );
    }

    #[test]
    fn test_compound_strictest_wins() {
        // deny beats ask
        assert_eq!(
            evaluate_command("git status && git push --force")
                .unwrap()
                .0,
            "deny"
        );
        // ask beats pass-through
        assert_eq!(
            evaluate_command("git status && git push origin main")
                .unwrap()
                .0,
            "ask"
        );
        // all safe passes through
        assert!(evaluate_command("git status && git diff").is_none());
    }

    #[test]
    fn test_shell_wrapper_depth_limit() {
        // Direct command works at max depth
        assert!(matches!(
            evaluate_command_at_depth("git push --force", MAX_SHELL_DEPTH),
            Some(("deny", _))
        ));
        // One wrapping level at max depth: inner exceeds limit, passes through
        assert!(evaluate_command_at_depth("bash -c 'git push --force'", MAX_SHELL_DEPTH).is_none());
    }

    #[test]
    fn test_combined_short_flags_git() {
        // git push: combined flags containing -f
        assert!(matches!(
            process_segment_at_depth("git push -vf origin main", 0),
            Some(("deny", _))
        ));
        assert!(matches!(
            process_segment_at_depth("git push -nf", 0),
            Some(("deny", _))
        ));
        // git clean: -fd is the canonical invocation
        assert!(matches!(
            process_segment_at_depth("git clean -fd", 0),
            Some(("ask", _))
        ));
        assert!(matches!(
            process_segment_at_depth("git clean -fxd", 0),
            Some(("ask", _))
        ));
        // git add: combined flags containing -A
        assert!(matches!(
            process_segment_at_depth("git add -Av", 0),
            Some(("deny", _))
        ));
        // git branch: combined flags containing -D
        assert!(matches!(
            process_segment_at_depth("git branch -Dv feature", 0),
            Some(("ask", _))
        ));
    }

    #[test]
    fn test_end_of_options_separator() {
        // After --, tokens are positional arguments, not flags.
        // rm -r -- -forceful-name should be "ask" (recursive) not "deny" (recursive + force)
        assert!(matches!(
            process_segment_at_depth("rm -r -- -forceful-name", 0),
            Some(("ask", _))
        ));
        // git clean -- -f: the -f is after --, so clean has no force flag → passes through
        assert!(process_segment_at_depth("git clean -- -f", 0).is_none());
    }

    #[test]
    fn test_strip_git_flags_respects_separator() {
        // -C after -- should not be stripped
        let tokens: Vec<String> = vec!["git", "push", "--", "-C"]
            .into_iter()
            .map(String::from)
            .collect();
        let stripped = strip_git_flags(&tokens);
        assert_eq!(stripped, vec!["git", "push", "--", "-C"]);
    }

    #[test]
    fn test_docker_prune_positional() {
        // "prune" as an argument value or image name should not trigger the prune rule
        assert!(process_segment_at_depth("docker build --tag prune .", 0).is_none());
        assert!(process_segment_at_depth("docker run --name prune-test nginx", 0).is_none());
        assert!(process_segment_at_depth("docker run prune", 0).is_none());
        assert!(process_segment_at_depth("docker exec prune ls", 0).is_none());
        // Actual prune subcommands still work
        assert!(matches!(
            process_segment_at_depth("docker image prune", 0),
            Some(("ask", _))
        ));
        assert!(matches!(
            process_segment_at_depth("docker container prune", 0),
            Some(("ask", _))
        ));
    }
}
