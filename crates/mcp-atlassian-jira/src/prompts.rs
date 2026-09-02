//! Jira prompts (D30).
//!
//! A prompt is a user-invoked template — in most clients a slash command, so
//! `jira_issue` is typed as `/jira_issue PROJ-123`. Unlike a tool, the model
//! does not choose it; the user does.
//!
//! These fetch their data rather than instructing the model to fetch it. A
//! prompt that only says "call jira_get_issue on PROJ-123" saves the user
//! nothing they could not type, and costs a round trip before the model has
//! seen anything.

use rmcp::{
    handler::server::{router::prompt::PromptRouter, wrapper::Parameters},
    model::{PromptMessage, Role},
    prompt, prompt_router, schemars, ErrorData as McpError,
};
use serde::Deserialize;

use mcp_atlassian_client::mcp::to_mcp_error;

use crate::models::{Comment, Issue};
use crate::{JiraTools, SearchParams};
use mcp_atlassian_client::query::quote;

/// Newest comments pulled into the briefing. Enough to carry the discussion
/// that changed the ticket, short of pasting a year of it.
const COMMENTS: u32 = 5;
/// Character budgets, so one enormous description cannot crowd out everything
/// after it. The model can call `jira_get_issue` for the untruncated text.
const MAX_DESCRIPTION: usize = 4000;
const MAX_COMMENT: usize = 800;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IssueArgs {
    /// Issue key, e.g. `PROJ-123`.
    pub issue_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TriageArgs {
    /// Project key, e.g. `PROJ`.
    pub project_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StandupArgs {
    /// Numeric board id (see jira_get_boards).
    pub board_id: u64,
}

/// Unresolved, unassigned issues pulled into a triage.
const TRIAGE_ISSUES: u32 = 25;
/// Issues of the active sprint pulled into a standup.
const STANDUP_ISSUES: u32 = 50;

#[prompt_router(router = "jira_prompts_router", vis = "pub(crate)")]
impl JiraTools {
    /// Brief on one Jira issue: pulls the issue and its newest comments, then
    /// asks for a plan. Use when someone names a ticket and wants to know
    /// what it is and what to do about it.
    #[prompt(name = "jira_issue")]
    pub async fn jira_issue_prompt(
        &self,
        Parameters(args): Parameters<IssueArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let key = args.issue_key.trim();
        let issue = self
            .client()
            .get_issue(key, None)
            .await
            .map_err(to_mcp_error)?;
        // Comments are a separate endpoint, and an issue with none is normal —
        // a failure here should not lose the briefing that already succeeded.
        let comments = match self.client().get_comments(key, COMMENTS, 0).await {
            Ok(page) => page.comments,
            Err(error) => {
                tracing::debug!(%key, %error, "comments unavailable for the issue briefing");
                Vec::new()
            }
        };
        Ok(vec![PromptMessage::new_text(
            Role::User,
            brief(&issue, &comments),
        )])
    }

    /// Triage a project's unassigned, unresolved issues: lists them by
    /// priority and asks for owners and an order. Use when a backlog needs
    /// sorting rather than one ticket.
    #[prompt(name = "jira_triage")]
    pub async fn jira_triage_prompt(
        &self,
        Parameters(args): Parameters<TriageArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let project = args.project_key.trim().to_ascii_uppercase();
        let page = self
            .client()
            .search(&SearchParams {
                jql: format!(
                    "project = {} AND assignee is EMPTY AND resolution is EMPTY \
                     ORDER BY priority DESC, created ASC",
                    quote(&project)
                ),
                max_results: TRIAGE_ISSUES,
                ..Default::default()
            })
            .await
            .map_err(to_mcp_error)?;
        Ok(vec![PromptMessage::new_text(
            Role::User,
            triage(&project, &page.issues, page.total),
        )])
    }

    /// Standup for a board's active sprint: the sprint's issues grouped by
    /// status, then the ask. Use at the start of a day or a sync.
    #[prompt(name = "jira_standup")]
    pub async fn jira_standup_prompt(
        &self,
        Parameters(args): Parameters<StandupArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let sprints = self
            .client()
            .get_sprints(args.board_id, Some("active"), 1, 0)
            .await
            .map_err(to_mcp_error)?;
        let Some(sprint) = sprints.values.into_iter().next() else {
            return Err(McpError::invalid_params(
                format!(
                    "board {} has no active sprint; list its sprints with jira_get_sprints",
                    args.board_id
                ),
                None,
            ));
        };
        let page = self
            .client()
            .get_sprint_issues(sprint.id, STANDUP_ISSUES, 0)
            .await
            .map_err(to_mcp_error)?;
        Ok(vec![PromptMessage::new_text(
            Role::User,
            standup(&sprint, &page.issues, page.total),
        )])
    }
}

/// One issue on one line: key, type, priority, assignee, summary.
fn line(issue: &Issue) -> String {
    let f = &issue.fields;
    format!(
        "- {} [{}] {} — {}{}",
        issue.key,
        unset(f.issuetype.as_ref().map(|t| t.name.as_str())),
        unset(f.priority.as_ref().map(|p| p.name.as_str())),
        f.summary.as_deref().unwrap_or("(no summary)"),
        f.assignee
            .as_ref()
            .map(|u| format!(" ({})", u.display_name))
            .unwrap_or_default(),
    )
}

/// A note when the page did not hold everything.
fn more(shown: usize, total: Option<u64>) -> String {
    match total {
        Some(total) if total as usize > shown => {
            format!("\n({total} in total; the first {shown} are listed)\n")
        }
        _ => String::new(),
    }
}

fn triage(project: &str, issues: &[Issue], total: Option<u64>) -> String {
    let mut out = format!("Triage: unassigned, unresolved issues in {project}\n\n");
    if issues.is_empty() {
        out.push_str("Nothing to triage — every open issue has an assignee.\n");
    } else {
        for issue in issues {
            out.push_str(&line(issue));
            out.push('\n');
        }
        out.push_str(&more(issues.len(), total));
    }
    out.push_str(
        "\n---\n\
         Triage these:\n\
         1. Group them: bugs blocking users, work that unblocks other work, the rest.\n\
         2. For the first group, propose an owner — use jira_search_assignable_users to \
         find who can take issues in this project — and a priority if the current one \
         is wrong.\n\
         3. Name anything that looks like a duplicate or is missing the detail needed \
         to act on it.\n\n\
         Do not assign, transition or edit anything unless asked to.\n",
    );
    out
}

fn standup(sprint: &crate::models::Sprint, issues: &[Issue], total: Option<u64>) -> String {
    let mut out = format!("Standup: sprint \"{}\" ({})", sprint.name, sprint.state);
    if let Some(end) = &sprint.end_date {
        out.push_str(&format!(", ends {end}"));
    }
    out.push('\n');
    if let Some(goal) = sprint.goal.as_deref().filter(|g| !g.trim().is_empty()) {
        out.push_str(&format!("Goal: {}\n", goal.trim()));
    }
    if issues.is_empty() {
        out.push_str("\nThe sprint has no issues.\n");
    } else {
        // Grouped by status, in the order the statuses first appear — which
        // for a board is roughly the workflow order.
        let mut groups: Vec<(String, Vec<&Issue>)> = Vec::new();
        for issue in issues {
            let status = unset(issue.fields.status.as_ref().map(|s| s.name.as_str())).to_string();
            match groups.iter_mut().find(|(name, _)| *name == status) {
                Some((_, members)) => members.push(issue),
                None => groups.push((status, vec![issue])),
            }
        }
        for (status, members) in &groups {
            out.push_str(&format!("\n{status} ({})\n", members.len()));
            for issue in members {
                out.push_str(&line(issue));
                out.push('\n');
            }
        }
        out.push_str(&more(issues.len(), total));
    }
    out.push_str(
        "\n---\n\
         Run the standup:\n\
         1. Say in three lines where the sprint stands against its goal.\n\
         2. List what is blocked or has not moved — call jira_get_changelog on an issue \
         if you need to know when it last changed.\n\
         3. Name what is at risk of missing the sprint end and what to do about it.\n\n\
         Do not move, assign or transition anything unless asked to.\n",
    );
    out
}

/// All Jira prompt routes.
pub fn router() -> PromptRouter<JiraTools> {
    JiraTools::jira_prompts_router()
}

/// Renders the briefing: the facts first, the ask last.
fn brief(issue: &Issue, comments: &[Comment]) -> String {
    let fields = &issue.fields;
    let mut out = format!(
        "Jira issue {} — {}\n\n",
        issue.key,
        fields.summary.as_deref().unwrap_or("(no summary)")
    );

    out.push_str(&format!(
        "Type: {}\nStatus: {}\nPriority: {}\nAssignee: {}\nReporter: {}\n",
        unset(fields.issuetype.as_ref().map(|t| t.name.as_str())),
        unset(fields.status.as_ref().map(|s| s.name.as_str())),
        unset(fields.priority.as_ref().map(|p| p.name.as_str())),
        nobody(fields.assignee.as_ref().map(|u| u.display_name.as_str())),
        nobody(fields.reporter.as_ref().map(|u| u.display_name.as_str())),
    ));
    if !fields.labels.is_empty() {
        out.push_str(&format!("Labels: {}\n", fields.labels.join(", ")));
    }
    if let Some(parent) = &fields.parent {
        out.push_str(&format!("Parent: {}\n", linked(parent)));
    }
    if !fields.subtasks.is_empty() {
        let subtasks: Vec<String> = fields.subtasks.iter().map(linked).collect();
        out.push_str(&format!("Subtasks: {}\n", subtasks.join("; ")));
    }
    for link in &fields.issuelinks {
        // Phrase the link from this issue's side: "blocks PROJ-3".
        let (verb, other) = match (&link.outward_issue, &link.inward_issue) {
            (Some(other), _) => (&link.link_type.outward, other),
            (None, Some(other)) => (&link.link_type.inward, other),
            (None, None) => continue,
        };
        out.push_str(&format!("Link: {verb} {}\n", linked(other)));
    }
    if let Some(updated) = &fields.updated {
        out.push_str(&format!("Updated: {updated}\n"));
    }

    out.push_str("\nDescription\n");
    match fields.description.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => {
            out.push_str(&truncate(text, MAX_DESCRIPTION));
            out.push('\n');
        }
        _ => out.push_str("(empty)\n"),
    }

    if comments.is_empty() {
        out.push_str("\nNo comments.\n");
    } else {
        out.push_str(&format!("\nNewest {} comment(s)\n", comments.len()));
        for comment in comments {
            let author = comment
                .author
                .as_ref()
                .map(|user| user.display_name.as_str())
                .unwrap_or("unknown");
            let when = comment.created.as_deref().unwrap_or("undated");
            out.push_str(&format!(
                "\n[{when}] {author}:\n{}\n",
                truncate(comment.body.trim(), MAX_COMMENT)
            ));
        }
    }

    out.push_str(
        "\n---\n\
         Work this issue:\n\
         1. State in two sentences what is actually being asked.\n\
         2. Name what is unclear, missing or blocking. Call jira_get_issue, \
         jira_get_comments or jira_search if the answer is in Jira.\n\
         3. Propose the next concrete step, and say which tool call would perform it.\n\n\
         Do not create, update or transition anything unless asked to.\n",
    );
    out
}

/// Absent value of a field that is not a person. One word for both was a
/// mistake visible on the first real ticket: "Priority: unassigned" reads as
/// a person's, and a priority is not assigned to anyone — it is simply unset.
fn unset(value: Option<&str>) -> &str {
    value.unwrap_or("none")
}

/// Absent person.
fn nobody(value: Option<&str>) -> &str {
    value.unwrap_or("unassigned")
}

/// `PROJ-9 (Epic, In Progress)` — enough to know whether to look at it.
fn linked(issue: &crate::models::LinkedIssue) -> String {
    let fields = issue.fields.as_ref();
    let summary = fields.and_then(|f| f.summary.as_deref());
    let status = fields
        .and_then(|f| f.status.as_ref())
        .map(|s| s.name.as_str());
    match (summary, status) {
        (Some(summary), Some(status)) => format!("{} ({summary}, {status})", issue.key),
        (Some(summary), None) => format!("{} ({summary})", issue.key),
        _ => issue.key.clone(),
    }
}

/// Cuts on a character boundary and says so — a silently clipped description
/// reads as a complete one.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!(
        "{kept}\n… truncated, {max} of {} characters",
        text.chars().count()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IssueFields, Named, User};

    fn user(name: &str) -> User {
        User {
            account_id: None,
            name: None,
            display_name: name.to_string(),
            email_address: None,
            active: None,
        }
    }

    fn issue() -> Issue {
        Issue {
            key: "PROJ-123".into(),
            id: "10001".into(),
            fields: IssueFields {
                summary: Some("Search times out on large projects".into()),
                description: Some("JQL search returns 504 above ~50k issues.".into()),
                status: Some(Named {
                    name: "In Progress".into(),
                }),
                priority: Some(Named {
                    name: "High".into(),
                }),
                issuetype: Some(Named { name: "Bug".into() }),
                assignee: Some(user("Jane Doe")),
                reporter: Some(user("John Smith")),
                labels: vec!["backend".into(), "performance".into()],
                created: Some("2026-08-01T10:00:00.000+0000".into()),
                updated: Some("2026-09-01T12:00:00.000+0000".into()),
                ..Default::default()
            },
        }
    }

    fn comment(body: &str) -> Comment {
        Comment {
            id: "1".into(),
            author: Some(user("Jane Doe")),
            body: body.into(),
            created: Some("2026-09-01T12:00:00.000+0000".into()),
        }
    }

    #[test]
    fn the_briefing_carries_the_facts_a_plan_needs() {
        let brief = brief(&issue(), &[comment("Reproduced on staging.")]);
        for expected in [
            "PROJ-123",
            "Search times out on large projects",
            "In Progress",
            "Jane Doe",
            "backend, performance",
            "504",
            "Reproduced on staging.",
        ] {
            assert!(brief.contains(expected), "missing {expected:?}:\n{brief}");
        }
    }

    #[test]
    fn the_briefing_ends_with_the_ask_not_the_data() {
        let brief = brief(&issue(), &[]);
        assert!(brief.contains("Work this issue"), "{brief}");
        // A briefing is a read; nothing here should invite a write.
        assert!(
            brief.contains("Do not create, update or transition"),
            "{brief}"
        );
    }

    #[test]
    fn an_empty_issue_still_renders() {
        // Search-shaped responses carry only the requested fields (D4), and a
        // fresh ticket has no description and no comments.
        let mut issue = issue();
        issue.fields.description = None;
        issue.fields.assignee = None;
        issue.fields.priority = None;
        issue.fields.labels.clear();
        let brief = brief(&issue, &[]);
        assert!(brief.contains("(empty)"), "{brief}");
        assert!(brief.contains("No comments."), "{brief}");
        assert!(brief.contains("Assignee: unassigned"), "{brief}");
        // Not "unassigned": nobody assigns a priority to anyone.
        assert!(brief.contains("Priority: none"), "{brief}");
    }

    #[test]
    fn a_triage_lists_issues_by_line_and_says_when_there_are_more() {
        let text = triage("PROJ", &[issue()], Some(40));
        assert!(
            text.contains("- PROJ-123 [Bug] High — Search times out"),
            "{text}"
        );
        assert!(text.contains("40 in total"), "{text}");
        assert!(text.contains("Do not assign"), "{text}");
        assert!(triage("PROJ", &[], None).contains("Nothing to triage"));
    }

    #[test]
    fn a_standup_groups_by_status_in_order_of_appearance() {
        let sprint = crate::models::Sprint {
            id: 3,
            name: "Sprint 3".into(),
            state: "active".into(),
            start_date: None,
            end_date: Some("2026-09-12".into()),
            goal: Some("Ship search".into()),
        };
        let mut done = issue();
        done.key = "PROJ-2".into();
        done.fields.status = Some(Named {
            name: "Done".into(),
        });
        let text = standup(&sprint, &[issue(), done, issue()], None);
        assert!(text.contains("Goal: Ship search"), "{text}");
        assert!(text.contains("ends 2026-09-12"), "{text}");
        let in_progress = text.find("In Progress (2)").expect("group");
        let finished = text.find("Done (1)").expect("group");
        assert!(in_progress < finished, "{text}");
        assert!(text.contains("Run the standup"), "{text}");
    }

    #[test]
    fn an_enormous_description_is_cut_and_says_so() {
        let mut issue = issue();
        issue.fields.description = Some("x".repeat(MAX_DESCRIPTION + 500));
        let brief = brief(&issue, &[]);
        assert!(brief.contains("… truncated"), "{brief}");
        assert!(brief.len() < MAX_DESCRIPTION + 2000, "{}", brief.len());
    }
}
