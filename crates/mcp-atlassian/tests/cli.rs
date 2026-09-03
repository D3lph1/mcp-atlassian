//! The flags, exercised through the built binary.
//!
//! `--version`, `--help`, `--list-tools` and `--completions` all have to work
//! before any configuration exists — that is most of their point — so these
//! run the executable with an empty environment rather than calling the
//! library functions behind them. Calling the functions would pass even if
//! `main` never wired the flag up.

use std::process::Command;

use mcp_atlassian::completions::{render, FLAGS, SHELLS};

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

    let (ok, out) = run(&["--list-tools"]);
    assert!(ok, "{out}");
    assert!(out.contains("jira_get_issue"), "{out}");
}

#[test]
fn every_shell_gets_a_script_and_anything_else_gets_an_error() {
    for shell in SHELLS {
        let (ok, out) = run(&["--completions", shell]);
        assert!(ok, "{shell}: {out}");
        assert!(
            out.contains("mcp-atlassian"),
            "{shell}: the script should mention the command: {out}"
        );
    }

    // Naming the shells is the difference between a usable error and a wall.
    let (ok, out) = run(&["--completions", "tcsh"]);
    assert!(!ok, "an unknown shell should fail: {out}");
    for shell in SHELLS {
        assert!(out.contains(shell), "the error should name {shell}: {out}");
    }

    let (ok, _) = run(&["--completions"]);
    assert!(!ok, "a missing shell argument should fail");
}

/// A flag added to the CLI but not to a completion script would leave users
/// unable to tab to it, and nothing else would catch that.
#[test]
fn every_flag_appears_in_every_script() {
    for shell in SHELLS {
        let script = render(shell).expect("a script for every listed shell");
        for (flag, _) in FLAGS {
            // fish spells flags without the leading dashes: `-l version`.
            let spelled = if shell == "fish" {
                flag.trim_start_matches("--").to_string()
            } else {
                flag.to_string()
            };
            assert!(
                script.contains(&spelled),
                "{shell}: {flag} is missing from the completion script"
            );
        }
    }
}

/// The help text and the completions describe the same command; a flag in one
/// and not the other is a contradiction a user meets as a surprise.
#[test]
fn the_help_text_and_the_flag_list_agree() {
    let (_, help) = run(&["--help"]);
    for (flag, _) in FLAGS {
        assert!(help.contains(flag), "{flag} is not in --help:\n{help}");
    }
}
