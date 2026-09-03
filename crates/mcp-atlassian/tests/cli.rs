//! The command line, exercised through the built binary.
//!
//! `--version`, `--help`, `tools` and `completions` all have to work before
//! any configuration exists — that is most of their point — so these run the
//! executable with an empty environment rather than calling the library
//! functions behind them. Calling the functions would pass even if `main`
//! never wired the command up.

use std::process::Command;

use clap::{CommandFactory, Parser, ValueEnum};
use clap_complete::Shell;
use mcp_atlassian::cli::{Cli, Command as CliCommand};

/// The binary cargo just built, run with nothing in the environment.
fn run(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_mcp-atlassian"))
        .args(args)
        .env_clear()
        .output()
        .expect("the binary should be runnable");
    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    (output.status.success(), text)
}

#[test]
fn the_commands_work_without_any_configuration() {
    let (ok, out) = run(&["--version"]);
    assert!(ok, "{out}");
    assert!(out.contains(env!("CARGO_PKG_VERSION")), "{out}");

    let (ok, out) = run(&["--help"]);
    assert!(ok, "{out}");
    for command in ["serve", "tools", "completions"] {
        assert!(out.contains(command), "{command} is not in --help:\n{out}");
    }
    // Configuration is documented once, under `serve`; the top level points
    // there rather than repeating a list that would drift.
    assert!(out.contains("serve --help"), "{out}");

    let (ok, out) = run(&["tools"]);
    assert!(ok, "{out}");
    assert!(out.contains("jira_get_issue"), "{out}");

    // Each command has its own help, with its own options.
    let (ok, out) = run(&["tools", "--help"]);
    assert!(ok, "{out}");
    assert!(out.contains("--format"), "{out}");
}

/// No command means `serve`, so an MCP client's configuration — which runs
/// the bare binary — keeps working. Unconfigured, both forms fail the same
/// way: by naming the variables they wanted.
#[test]
fn no_command_and_serve_are_the_same_thing() {
    let (ok, bare) = run(&[]);
    assert!(!ok, "unconfigured, it should refuse to start: {bare}");
    assert!(bare.contains("JIRA_URL"), "{bare}");

    let (ok, serve) = run(&["serve"]);
    assert!(!ok, "{serve}");
    assert!(serve.contains("JIRA_URL"), "{serve}");
}

#[test]
fn the_catalogue_has_a_machine_readable_form() {
    let (ok, out) = run(&["tools", "--format", "json"]);
    assert!(ok, "{out}");
    let tools: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(tools.len(), 70, "{out}");
    let get_issue = tools
        .iter()
        .find(|t| t["name"] == "jira_get_issue")
        .expect("jira_get_issue listed");
    assert_eq!(get_issue["kind"], "read-only");
    assert_eq!(get_issue["product"], "jira");

    // --format belongs to `tools`; at the top level it is unknown.
    let (ok, out) = run(&["--format", "json"]);
    assert!(!ok, "{out}");
}

#[test]
fn every_shell_gets_a_script_and_anything_else_gets_an_error() {
    for shell in Shell::value_variants() {
        let name = shell.to_string();
        let (ok, out) = run(&["completions", &name]);
        assert!(ok, "{name}: {out}");
        assert!(
            out.contains("mcp-atlassian"),
            "{name}: the script should mention the command: {out}"
        );
    }

    // Naming the shells is the difference between a usable error and a wall.
    let (ok, out) = run(&["completions", "tcsh"]);
    assert!(!ok, "an unknown shell should fail: {out}");
    assert!(
        out.contains("zsh"),
        "the error should list the shells: {out}"
    );

    let (ok, _) = run(&["completions"]);
    assert!(!ok, "a missing shell argument should fail");
}

/// A typo gets a suggestion rather than a bare rejection.
#[test]
fn a_misspelled_command_is_corrected() {
    let (ok, out) = run(&["tool"]);
    assert!(!ok, "{out}");
    assert!(out.contains("tools"), "{out}");
}

/// Completions are generated from the same declarations as the help, so every
/// subcommand and every long option the parser knows must appear in every
/// script — one missing is invisible rather than broken.
#[test]
fn every_command_and_option_appears_in_every_script() {
    let command = Cli::command();
    let mut names: Vec<String> = command
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .map(|c| c.get_name().to_string())
        .collect();
    names.extend(
        command
            .get_subcommands()
            .flat_map(|c| c.get_arguments())
            .filter_map(|a| a.get_long())
            .map(|l| l.to_string()),
    );
    assert!(names.contains(&"tools".to_string()), "{names:?}");
    assert!(names.contains(&"format".to_string()), "{names:?}");

    for shell in Shell::value_variants() {
        let script = Cli::completion_script(*shell);
        for name in &names {
            assert!(
                script.contains(name.as_str()),
                "{shell}: `{name}` is missing from the completion script"
            );
        }
    }
}

/// The flags on `serve` stand in for environment variables, so what matters
/// is that each maps to the spelling `Config::read` expects. Parsed here
/// rather than run, because a typo in the mapping is invisible from outside.
fn overrides_for(args: &[&str]) -> Vec<(&'static str, String)> {
    let mut argv = vec!["mcp-atlassian", "serve"];
    argv.extend_from_slice(args);
    match Cli::try_parse_from(argv).expect("parses").command {
        Some(CliCommand::Serve(serve)) => serve.overrides(),
        other => panic!("expected serve, got {other:?}"),
    }
}

#[test]
fn a_flag_stands_in_for_its_variable() {
    for (flag, value, variable) in [
        ("--jira-url", "https://x.atlassian.net", "JIRA_URL"),
        (
            "--confluence-url",
            "https://x.atlassian.net/wiki",
            "CONFLUENCE_URL",
        ),
        ("--jira-deployment", "server", "JIRA_DEPLOYMENT"),
        ("--enabled-tools", "jira_*", "ENABLED_TOOLS"),
        ("--disabled-tools", "*_delete_*", "DISABLED_TOOLS"),
        ("--audit-log", "/tmp/a.jsonl", "AUDIT_LOG_FILE"),
        ("--attachment-dir", "/tmp", "ATTACHMENT_DIR"),
        ("--cache-ttl", "60", "CACHE_TTL"),
        ("--request-timeout", "10", "REQUEST_TIMEOUT"),
        ("--log-filter", "debug", "LOG_FILTER"),
        ("--transport", "streamable-http", "TRANSPORT"),
        ("--port", "9000", "PORT"),
        ("--jira-api-token-file", "/run/s", "JIRA_API_TOKEN_FILE"),
        ("--oauth-client-id", "abc", "ATLASSIAN_OAUTH_CLIENT_ID"),
    ] {
        let overrides = overrides_for(&[flag, value]);
        assert_eq!(
            overrides,
            vec![(variable, value.to_string())],
            "{flag} should stand in for {variable}"
        );
    }
}

/// A switch left off must say nothing, or it would overwrite the variable
/// with `false` — and the variable understands spellings clap does not.
#[test]
fn a_switch_left_off_overrides_nothing() {
    assert!(overrides_for(&[]).is_empty());
    assert_eq!(
        overrides_for(&["--read-only"]),
        vec![("READ_ONLY", "true".to_string())]
    );
    assert_eq!(
        overrides_for(&["--dry-run", "--confirm-destructive", "--no-banner"]).len(),
        3
    );
}

/// Arguments are visible in `ps` and land in shell history, so no secret may
/// be one — only the file a token is read from (D28).
#[test]
fn no_secret_can_be_passed_as_a_flag() {
    for secret in [
        "--jira-api-token",
        "--jira-personal-token",
        "--confluence-api-token",
        "--confluence-personal-token",
        "--oauth-client-secret",
        "--oauth-refresh-token",
        "--mcp-bearer-token",
    ] {
        let (ok, out) = run(&["serve", secret, "value"]);
        assert!(!ok, "{secret} must not be accepted: {out}");
    }

    // The file each is read from is a flag, because a path is not a secret.
    let (ok, out) = run(&["serve", "--help"]);
    assert!(ok, "{out}");
    for flag in [
        "--jira-api-token-file",
        "--jira-personal-token-file",
        "--oauth-client-secret-file",
        "--mcp-bearer-token-file",
    ] {
        assert!(out.contains(flag), "{flag} is not in `serve --help`");
    }
}

/// `serve --help` is the reference now, so it has to name the variable behind
/// each flag — otherwise the two documents drift.
#[test]
fn serve_help_names_the_variable_behind_each_flag() {
    let (ok, out) = run(&["serve", "--help"]);
    assert!(ok, "{out}");
    for variable in [
        "JIRA_URL",
        "CONFLUENCE_URL",
        "READ_ONLY",
        "DRY_RUN",
        "ENABLED_TOOLS",
        "AUDIT_LOG_FILE",
        "ATTACHMENT_DIR",
        "CACHE_TTL",
        "LOG_FILTER",
        "TRANSPORT",
        "MCP_BEARER_TOKEN_FILE",
    ] {
        assert!(
            out.contains(variable),
            "{variable} is not in `serve --help`"
        );
    }
}

/// Configuration is validated in one place, and it names settings by their
/// variable. Now that each has a flag, a failure has to point at them —
/// otherwise someone who typed `serve` never learns the flags exist.
#[test]
fn a_configuration_failure_points_at_the_flags() {
    let (ok, out) = run(&["serve"]);
    assert!(!ok, "{out}");
    assert!(out.contains("serve --help"), "no hint about flags:\n{out}");
    assert!(out.contains("JIRA_URL"), "{out}");

    // A partly configured service says what is missing, not just that
    // something is.
    let (ok, out) = run(&["serve", "--jira-url", "https://x.atlassian.net"]);
    assert!(!ok, "{out}");
    assert!(out.contains("JIRA_PERSONAL_TOKEN"), "{out}");
    assert!(out.contains("serve --help"), "{out}");
}
