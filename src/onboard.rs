use anyhow::{bail, Result};
use std::path::Path;

use crate::goal_edit::{save_goal, GoalForm};
use crate::install::install_for_user;

/// `repovow onboard` — init + set goal in one step (avoids empty `.repovow`).
pub fn run_onboard(
    title: &str,
    accept: Vec<String>,
    constraint: Vec<String>,
    step: Option<String>,
    root: Option<&Path>,
) -> Result<()> {
    let title = title.trim();
    if title.is_empty() {
        bail!(
            "Goal title is required.\n\nExample:\n  \
             repovow onboard \"Ship auth\" --accept \"tests pass\" --step \"scaffold routes\""
        );
    }

    let project = install_for_user(root)?;
    let form = GoalForm {
        title: title.to_string(),
        step: step.unwrap_or_default(),
        acceptance: accept,
        constraints: constraint,
    };
    save_goal(&form, Some(&project), "onboard")?;

    println!("RepoVow onboard complete in {}", project.display());
    println!("Goal: {title}");
    println!("Hooks: persistent Claude/Codex routers + project-local fallback hooks");
    println!("State: {}/snapshot.md", project.join(".repovow").display());
    println!("\nClaude applies the router live. RepoVow registers trust for its own Codex hooks; new projects then activate without per-repository setup.");
    Ok(())
}
