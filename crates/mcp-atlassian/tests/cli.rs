//! The flags, exercised through the built binary.
//!
//! `--version`, `--help`, `--list-tools` and `--completions` all have to work
//! before any configuration exists — that is most of their point — so these
//! run the executable with an empty environment rather than calling the
//! library functions behind them. Calling the functions would pass even if
//! `main` never wired the flag up.

use std::process::Command;

use clap::{CommandFactory, ValueEnum};
use clap_complete::Shell;
use mcp_atlassian::cli::Cli;

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
fn the_flags_work_without_any_configuration() {
    let (ok, out) = run(&["--version"]);
    assert!(ok, "{out}");
    assert!(out.contains(env!("CARGO_PKG_VERSION")), "{out}");

    let (ok, out) = run(&["--help"]);
    assert!(ok, "{out}");
    assert!(out.contains("--list-tools"), "{out}");
    assert!(out.contains("--completions"), "{out}");
    // The help is where a user looks for configuration; it must name it.
    assert!(out.contains("JIRA_URL"), "{out}");

    let (ok, out) = run(&["--list-tools"]);
    assert!(ok, "{out}");
    assert!(out.contains("jira_get_issue"), "{out}");
}

#[test]
fn the_catalogue_has_a_machine_readable_form() {
    let (ok, out) = run(&["--list-tools", "--format", "json"]);
    assert!(ok, "{out}");
    let tools: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(tools.len(), 70, "{out}");
    let get_issue = tools
        .iter()
        .find(|t| t["name"] == "jira_get_issue")
        .expect("jira_get_issue listed");
    assert_eq!(get_issue["kind"], "read-only");
    assert_eq!(get_issue["product"], "jira");

    // --format without --list-tools is a mistake, and clap says so.
    let (ok, out) = run(&["--format", "json"]);
    assert!(!ok, "{out}");
    assert!(out.contains("--list-tools"), "{out}");
}

#[test]
fn every_shell_gets_a_script_and_anything_else_gets_an_error() {
    for shell in Shell::value_variants() {
        let name = shell.to_string();
        let (ok, out) = run(&["--completions", &name]);
        assert!(ok, "{name}: {out}");
        assert!(
            out.contains("mcp-atlassian"),
            "{name}: the script should mention the command: {out}"
        );
    }

    // Naming the shells is the difference between a usable error and a wall.
    let (ok, out) = run(&["--completions", "tcsh"]);
    assert!(!ok, "an unknown shell should fail: {out}");
    assert!(
        out.contains("zsh"),
        "the error should list the shells: {out}"
    );

    let (ok, _) = run(&["--completions"]);
    assert!(!ok, "a missing shell argument should fail");
}

/// A typo gets a suggestion rather than a bare rejection.
#[test]
fn a_misspelled_flag_is_corrected() {
    let (ok, out) = run(&["--lst-tools"]);
    assert!(!ok, "{out}");
    assert!(out.contains("--list-tools"), "{out}");
}

/// Completions are generated from the same declarations as the help, so every
/// long flag the parser knows must appear in every script.
#[test]
fn every_flag_appears_in_every_script() {
    let command = Cli::command();
    let flags: Vec<String> = command
        .get_arguments()
        .filter_map(|a| a.get_long())
        .map(|l| format!("--{l}"))
        .collect();
    assert!(flags.contains(&"--list-tools".to_string()), "{flags:?}");

    for shell in Shell::value_variants() {
        let script = Cli::completion_script(*shell);
        for flag in &flags {
            // fish and PowerShell spell long flags without the dashes in
            // places; the bare name is the stable part.
            let bare = flag.trim_start_matches("--");
            assert!(
                script.contains(bare),
                "{shell}: {flag} is missing from the completion script"
            );
        }
    }
}
