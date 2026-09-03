//! The command line, exercised through the built binary.
//!
//! `--version`, `--help`, `tools` and `completions` all have to work before
//! any configuration exists — that is most of their point — so these run the
//! executable with an empty environment rather than calling the library
//! functions behind them. Calling the functions would pass even if `main`
//! never wired the command up.

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
fn the_commands_work_without_any_configuration() {
    let (ok, out) = run(&["--version"]);
    assert!(ok, "{out}");
    assert!(out.contains(env!("CARGO_PKG_VERSION")), "{out}");

    let (ok, out) = run(&["--help"]);
    assert!(ok, "{out}");
    for command in ["serve", "tools", "completions"] {
        assert!(out.contains(command), "{command} is not in --help:\n{out}");
    }
    // The help is where a user looks for configuration; it must name it.
    assert!(out.contains("JIRA_URL"), "{out}");

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
