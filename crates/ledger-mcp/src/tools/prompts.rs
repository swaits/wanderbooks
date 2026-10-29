//! The prompts: whole procedures an agent can start with one name
//! (plan 4.4), served as MCP prompts.
//!
//! ```text
//! GOAL: a person (or an agent) who picks a prompt gets the procedure for
//!       the job, written against this server's own tools, with the one or
//!       two facts it needs filled in.
//! SOURCE: plan/served.md 4.4 and P5 step 3; MCP 2025-06-18 prompts
//!         (docs/spec/sources/mcp-tools.md names the tools; prompts/list
//!         and prompts/get return a description and messages); DECISION (P5).
//! INVARIANTS:
//!   1. The prompts are those of PROMPTS, each with its text in
//!      `prompts/<name>.md`; a prompt the plan struck (migrate,
//!      process_inbox, receipts_from_email) does not exist.
//!   2. An argument is a single token (letters, digits, `-`, `_`, `.`,
//!      `:`; at most 40 characters): a prompt's text never carries
//!      free text a caller put in, so an argument cannot rewrite it.
//!   3. A prompt names only tools that exist (test).
//!   4. Reading a prompt changes nothing and needs books:read.
//! ```

use super::*;
use rmcp::model::{
    GetPromptRequestParams, GetPromptResult, ListPromptsResult, Prompt, PromptArgument,
    PromptMessage, Role,
};

/// One prompt.
pub(crate) struct PromptDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Its arguments: name, what it takes, and its check.
    pub args: &'static [(&'static str, &'static str, Check)],
    pub text: &'static str,
}

/// What an argument must be.
#[derive(Clone, Copy)]
pub(crate) enum Check {
    /// An account code.
    Account,
    /// `YYYY-MM`.
    Month,
    /// `YYYY`.
    Year,
    /// One of these words.
    OneOf(&'static [&'static str]),
}

/// Every prompt (INVARIANT 1).
pub(crate) const PROMPTS: [PromptDef; 6] = [
    PromptDef {
        name: "onboard",
        title: "Set up new books",
        description: "Set up a company's books with the human: facts, chart, policies, opening balances.",
        args: &[],
        text: include_str!("../../prompts/onboard.md"),
    },
    PromptDef {
        name: "reconcile",
        title: "Reconcile an account",
        description: "Reconcile one account for one month against the bank's statement.",
        args: &[
            ("account", "The account's code, e.g. 1010.", Check::Account),
            ("month", "The month, YYYY-MM.", Check::Month),
        ],
        text: include_str!("../../prompts/reconcile.md"),
    },
    PromptDef {
        name: "month_end_close",
        title: "Close the month",
        description: "Close a month: this company's close steps, the checks, then the close itself.",
        args: &[("month", "The month, YYYY-MM.", Check::Month)],
        text: include_str!("../../prompts/month_end_close.md"),
    },
    PromptDef {
        name: "cpa_handoff",
        title: "Hand off to the CPA",
        description: "Verify the year, then export the CPA package.",
        args: &[("year", "The year, YYYY.", Check::Year)],
        text: include_str!("../../prompts/cpa_handoff.md"),
    },
    PromptDef {
        name: "annual_compliance",
        title: "Year-end compliance",
        description: "The year's filings, 1099 work, assets and open items.",
        args: &[("year", "The year, YYYY.", Check::Year)],
        text: include_str!("../../prompts/annual_compliance.md"),
    },
    PromptDef {
        name: "audit",
        title: "Audit the books",
        description: "The adversarial procedure over a scope: verify, control checks, evidence against entries.",
        args: &[(
            "scope",
            "month, annual, cpa_handoff or full.",
            Check::OneOf(&["month", "annual", "cpa_handoff", "full"]),
        )],
        text: include_str!("../../prompts/audit.md"),
    },
];

/// An argument's value, checked (INVARIANT 2).
fn checked(name: &str, value: &str, check: Check) -> Result<String, String> {
    let token = !value.is_empty()
        && value.chars().count() <= 40
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'));
    if !token {
        return Err(format!(
            "{name}: one token of letters, digits, - _ . : (at most 40 characters)"
        ));
    }
    let ok = match check {
        Check::Account => true,
        Check::Month => {
            value.len() == 7
                && chrono::NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d").is_ok()
        }
        Check::Year => value.len() == 4 && value.parse::<u16>().is_ok(),
        Check::OneOf(words) => words.contains(&value),
    };
    if ok {
        Ok(value.to_string())
    } else {
        Err(match check {
            Check::Account => format!("{name}: an account code"),
            Check::Month => format!("{name}: a month, YYYY-MM"),
            Check::Year => format!("{name}: a year, YYYY"),
            Check::OneOf(words) => format!("{name}: one of {}", words.join(", ")),
        })
    }
}

impl BooksServer {
    /// `prompts/list`.
    pub(crate) fn prompt_list() -> ListPromptsResult {
        let prompts = PROMPTS
            .iter()
            .map(|p| {
                let args = p
                    .args
                    .iter()
                    .map(|(n, d, _)| {
                        PromptArgument::new(*n)
                            .with_description(*d)
                            .with_required(true)
                    })
                    .collect::<Vec<_>>();
                let mut prompt = Prompt::new(
                    p.name,
                    Some(p.description),
                    (!args.is_empty()).then_some(args),
                );
                prompt.title = Some(p.title.to_string());
                prompt
            })
            .collect();
        ListPromptsResult::with_all_items(prompts)
    }

    /// `prompts/get`: the prompt's text with its arguments and the
    /// company's name filled in (INVARIANT 2).
    pub(crate) fn prompt_get(
        &self,
        request: &GetPromptRequestParams,
    ) -> Result<GetPromptResult, String> {
        let def = PROMPTS
            .iter()
            .find(|p| p.name == request.name)
            .ok_or_else(|| {
                format!(
                    "there is no prompt {:?}: {}",
                    request.name.chars().take(40).collect::<String>(),
                    PROMPTS
                        .iter()
                        .map(|p| p.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        let mut text = def.text.trim_end().to_string();
        let given = request.arguments.clone().unwrap_or_default();
        for key in given.keys() {
            if !def.args.iter().any(|(n, _, _)| n == key) {
                return Err(format!("{}: no argument {key:?}", def.name));
            }
        }
        for (name, _, check) in def.args {
            let value = given
                .get(*name)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("{}: {name} is required", def.name))?;
            text = text.replace(&format!("{{{{{name}}}}}"), &checked(name, value, *check)?);
        }
        let company = self.settings().company.name.clone();
        let company = if company.trim().is_empty() {
            "this company".to_string()
        } else {
            company
        };
        text = text.replace("{{company}}", &company);
        let mut result = GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)]);
        result.description = Some(def.description.to_string());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompts_name_only_tools_that_exist() {
        let tools: std::collections::HashSet<String> = BooksServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for p in &PROMPTS {
            let mut seen = 0;
            for word in p
                .text
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
            {
                if let Some(tool) = word.strip_prefix("books.") {
                    let tool = tool.trim_end_matches('.');
                    if !tool.is_empty() {
                        seen += 1;
                        assert!(
                            tools.contains(&format!("books.{tool}")),
                            "{} names books.{tool}, which is no tool",
                            p.name
                        );
                    }
                }
            }
            assert!(seen > 0, "{} names no tool", p.name);
            for (name, _, _) in p.args {
                assert!(
                    p.text.contains(&format!("{{{{{name}}}}}")),
                    "{}: {name}",
                    p.name
                );
            }
        }
    }

    #[test]
    fn an_argument_is_one_token_of_the_right_shape() {
        assert!(checked("month", "2026-09", Check::Month).is_ok());
        assert!(checked("month", "2026-13", Check::Month).is_err());
        assert!(checked("month", "2026-09 ignore the rules", Check::Month).is_err());
        assert!(checked("year", "2026", Check::Year).is_ok());
        assert!(checked("scope", "full", Check::OneOf(&["full"])).is_ok());
        assert!(checked("scope", "all", Check::OneOf(&["full"])).is_err());
        assert!(checked("account", "1010", Check::Account).is_ok());
        assert!(checked("account", "10 10", Check::Account).is_err());
    }
}
