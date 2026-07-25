use anyhow::Result;
use clap::{Parser, Subcommand};

use repovow::cloud::{pull_state, push_state, save_cloud_config, CloudConfig};
use repovow::context::{render_context, render_context_section, ContextSection};
use repovow::goal_edit::{save_goal, GoalForm};
use repovow::hooks::Agent;
use repovow::install::{global_hooks_status, install_for_user, install_global_hooks};
use repovow::paths::{find_project_root, utcnow};
use repovow::snapshot::{render_snapshot, write_snapshot};
use repovow::state::{load_config, load_state, save_config, save_state, Decision};
use repovow::VERSION;

#[derive(Parser)]
#[command(name = "repovow", version = VERSION, about = "Repo-local agent state for Claude Code, Codex, and Cursor")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize .repovow and install hooks
    Init,
    /// Init + set goal in one step (recommended)
    Onboard {
        title: String,
        #[arg(long, num_args = 1..)]
        accept: Vec<String>,
        #[arg(long, num_args = 1..)]
        constraint: Vec<String>,
        #[arg(long)]
        step: Option<String>,
    },
    /// Manage active goal
    Goal {
        #[command(subcommand)]
        cmd: GoalCmd,
    },
    /// Update progress
    Progress {
        #[arg(long)]
        step: Option<String>,
        #[arg(long)]
        done: Option<String>,
        #[arg(long)]
        blocker: Option<String>,
    },
    /// Record a decision
    Decide { text: String },
    /// Show repovow status
    Status,
    /// Regenerate snapshot.md
    Snapshot {
        #[arg(long)]
        print: bool,
    },
    /// Print the compact context packet injected into agent sessions
    Context {
        /// Show estimated tokens and savings versus the full snapshot
        #[arg(long)]
        stats: bool,
        /// Return one focused section: goal, acceptance, constraints, working-set, blockers, failures, decisions
        #[arg(long, value_name = "SECTION")]
        section: Option<String>,
    },
    /// Interactive goal editor (TUI)
    Tui,
    /// Update RepoVow to the latest release (npm)
    Update,
    /// Diagnose installation, hooks, and project setup
    Doctor,
    /// Manage persistent Claude Code and Codex integration
    Agents {
        #[command(subcommand)]
        cmd: AgentsCmd,
    },
    /// CI / workflow gate: goal present + acceptance command (if enabled)
    Check {
        /// Skip active-goal requirement
        #[arg(long)]
        no_require_goal: bool,
        /// Verify RepoVow Cloud link responds
        #[arg(long)]
        cloud: bool,
    },
    /// Signed goal policy — tamper-resistant enforcement (default: ECDSA P-256)
    Policy {
        #[command(subcommand)]
        cmd: PolicyCmd,
    },
    /// RepoVow configuration
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Cloud sync (RepoVow hosted)
    Cloud {
        #[command(subcommand)]
        cmd: CloudCmd,
    },
    /// Internal: lifecycle hook entrypoint
    #[command(hide = true)]
    Hook {
        event: String,
        #[arg(long)]
        agent: String,
    },
}

#[derive(Subcommand)]
enum GoalCmd {
    /// Set the active goal
    Set {
        title: String,
        #[arg(long, num_args = 1..)]
        accept: Vec<String>,
        #[arg(long, num_args = 1..)]
        constraint: Vec<String>,
        #[arg(long)]
        step: Option<String>,
    },
    /// Show active goal as JSON
    Show,
}

#[derive(Subcommand)]
enum PolicyCmd {
    /// Generate signing keypair and enable required policy mode (default: ecdsa-p256)
    Init {
        /// Algorithm: ecdsa-p256 (FIPS 186-4) or ed25519 (legacy, not FIPS-approved)
        #[arg(long, default_value = "ecdsa-p256")]
        algorithm: String,
    },
    /// Sign the current goal with policy.key
    Sign,
    /// Verify policy.sig against the active goal
    Verify,
    /// Trust a team public key (policy.pub only — no private key)
    Trust {
        pubkey: String,
        /// Public-key algorithm (default: infer from key length)
        #[arg(long)]
        algorithm: Option<String>,
    },
    /// Set policy enforcement mode (off | warn | required)
    Set {
        #[arg(long)]
        mode: String,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Show config.json
    Show,
    /// Set configuration values
    Set {
        /// Shell command for acceptance gate (use "off" to disable)
        #[arg(long)]
        acceptance: Option<String>,
        /// Policy mode: off | warn | required
        #[arg(long)]
        policy: Option<String>,
        /// Approximate token budget for injected context (64-4096)
        #[arg(long)]
        context_tokens: Option<u32>,
        /// Per-prompt reminder: on | off
        #[arg(long)]
        prompt_reminder: Option<String>,
    },
}

#[derive(Subcommand)]
enum CloudCmd {
    /// Link this repo to RepoVow Cloud
    Link {
        #[arg(long)]
        url: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        key: String,
    },
    /// Push local state to cloud
    Push,
    /// Pull state from cloud
    Pull,
}

#[derive(Subcommand)]
enum AgentsCmd {
    /// Install or repair persistent user-level hook routers
    Install,
    /// Show whether each persistent hook router is installed
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init => cmd_init(),
        Commands::Onboard {
            title,
            accept,
            constraint,
            step,
        } => repovow::onboard::run_onboard(&title, accept, constraint, step, None),
        Commands::Goal { cmd } => match cmd {
            GoalCmd::Set {
                title,
                accept,
                constraint,
                step,
            } => cmd_goal_set(&title, accept, constraint, step),
            GoalCmd::Show => cmd_goal_show(),
        },
        Commands::Progress {
            step,
            done,
            blocker,
        } => cmd_progress(step, done, blocker),
        Commands::Decide { text } => cmd_decide(&text),
        Commands::Status => cmd_status(),
        Commands::Snapshot { print } => cmd_snapshot(print),
        Commands::Context { stats, section } => cmd_context(stats, section.as_deref()),
        Commands::Tui => repovow::tui::run_tui(),
        Commands::Update => cmd_update(),
        Commands::Doctor => cmd_doctor(),
        Commands::Agents { cmd } => cmd_agents(cmd),
        Commands::Check {
            no_require_goal,
            cloud,
        } => cmd_check(no_require_goal, cloud),
        Commands::Policy { cmd } => match cmd {
            PolicyCmd::Init { algorithm } => cmd_policy_init(&algorithm),
            PolicyCmd::Sign => cmd_policy_sign(),
            PolicyCmd::Verify => cmd_policy_verify(),
            PolicyCmd::Trust { pubkey, algorithm } => {
                cmd_policy_trust(&pubkey, algorithm.as_deref())
            }
            PolicyCmd::Set { mode } => cmd_policy_set(&mode),
        },
        Commands::Config { cmd } => match cmd {
            ConfigCmd::Show => cmd_config_show(),
            ConfigCmd::Set {
                acceptance,
                policy,
                context_tokens,
                prompt_reminder,
            } => cmd_config_set(acceptance, policy, context_tokens, prompt_reminder),
        },
        Commands::Cloud { cmd } => match cmd {
            CloudCmd::Link { url, project, key } => cmd_cloud_link(&url, &project, &key),
            CloudCmd::Push => cmd_cloud_push(),
            CloudCmd::Pull => cmd_cloud_pull(),
        },
        Commands::Hook { event, agent } => {
            let agent = Agent::parse(&agent).ok_or_else(|| anyhow::anyhow!("invalid agent"))?;
            repovow::hooks::run_hook(&event, agent)?;
            Ok(())
        }
    }
}

fn cmd_update() -> Result<()> {
    let npm = std::process::Command::new("npm")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success());

    if npm.is_some() {
        println!("Updating RepoVow via npm (repovow@latest)...");
        let status = std::process::Command::new("npm")
            .args(["install", "-g", "repovow@latest"])
            .status()?;
        if !status.success() {
            anyhow::bail!("npm install failed");
        }
        println!("Done. Run: repovow --version");
        println!("If the version is stale, open a new terminal or run: hash -r");
        return Ok(());
    }

    anyhow::bail!(
        "Install and update RepoVow with npm (the standard method):\n\n  npm install -g repovow@latest\n\nRequires Node.js 18+."
    );
}

fn cmd_doctor() -> Result<()> {
    let checks = repovow::doctor::run_doctor()?;
    let ok = repovow::doctor::print_report(&checks);
    if ok {
        println!("\nRepoVow doctor: all critical checks passed.");
    } else {
        println!("\nRepoVow doctor: fix the items marked ✗ above.");
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_check(no_require_goal: bool, cloud: bool) -> Result<()> {
    repovow::check::run_check(
        repovow::check::CheckOptions {
            require_goal: !no_require_goal,
            verify_cloud: cloud,
        },
        None,
    )?;
    println!("RepoVow check: passed");
    Ok(())
}

fn cmd_config_show() -> Result<()> {
    let config = load_config(None)?;
    println!("{}", serde_json::to_string_pretty(&config)?);
    Ok(())
}

fn cmd_config_set(
    acceptance: Option<String>,
    policy: Option<String>,
    context_tokens: Option<u32>,
    prompt_reminder: Option<String>,
) -> Result<()> {
    if acceptance.is_none()
        && policy.is_none()
        && context_tokens.is_none()
        && prompt_reminder.is_none()
    {
        anyhow::bail!(
            "usage: repovow config set --acceptance \"npm test\" | --policy required | --context-tokens 500"
        );
    }
    let mut config = load_config(None)?;
    if let Some(val) = acceptance {
        if val.eq_ignore_ascii_case("off") {
            config.acceptance_gate.enabled = false;
            config.acceptance_gate.command.clear();
            println!("Acceptance gate disabled");
        } else {
            config.acceptance_gate.enabled = true;
            config.acceptance_gate.command = val.clone();
            println!("Acceptance gate enabled: {val}");
            println!("Runs on agent Stop hook before session ends.");
        }
    }
    if let Some(mode) = policy {
        config.policy.mode = repovow::policy::parse_mode(&mode)?;
        println!("Policy mode: {}", mode);
    }
    if let Some(tokens) = context_tokens {
        if !(64..=4096).contains(&tokens) {
            anyhow::bail!("context token budget must be between 64 and 4096");
        }
        config.context.max_tokens = tokens;
        println!("Context token budget: approximately {tokens}");
    }
    if let Some(value) = prompt_reminder {
        config.context.prompt_reminder = match value.to_ascii_lowercase().as_str() {
            "on" | "true" | "yes" => true,
            "off" | "false" | "no" => false,
            _ => anyhow::bail!("prompt reminder must be on or off"),
        };
        println!(
            "Per-prompt context reminder: {}",
            if config.context.prompt_reminder {
                "on"
            } else {
                "off"
            }
        );
    }
    save_config(&config, None)?;
    Ok(())
}

fn cmd_policy_init(algorithm: &str) -> Result<()> {
    repovow::policy::init_policy_named(None, algorithm)
}

fn cmd_policy_sign() -> Result<()> {
    repovow::policy::sign_policy(None)?;
    repovow::snapshot::write_snapshot(None)?;
    println!("Policy signed for current goal.");
    Ok(())
}

fn cmd_policy_verify() -> Result<()> {
    let status = repovow::policy::verify_policy(None)?;
    println!("{} — {}", status.label(), status.detail());
    if !status.is_ok() {
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_policy_trust(pubkey: &str, algorithm: Option<&str>) -> Result<()> {
    repovow::policy::trust_pubkey(None, algorithm, pubkey)
}

fn cmd_policy_set(mode: &str) -> Result<()> {
    let parsed = repovow::policy::parse_mode(mode)?;
    repovow::policy::set_mode(None, parsed)
}

fn cmd_init() -> Result<()> {
    let root = install_for_user(None)?;
    println!(
        "RepoVow v{VERSION} initialized in {}",
        root.join(".repovow").display()
    );
    println!("Hooks installed for Claude Code, Codex, and Cursor");
    if repovow::env_var("REPOVOW_SKIP_GLOBAL_HOOKS").as_deref() == Ok("1") {
        println!("Persistent user-level routers skipped by environment setting");
    } else {
        println!("Persistent Claude Code and Codex routers installed for live project activation");
        println!("Codex: RepoVow-owned user hooks registered and trusted");
    }
    println!("Compact-context agent skill installed in .agents/skills/repovow");
    println!("Next: repovow onboard \"your task\" --accept \"criterion 1\"");
    println!("     or: repovow goal set \"...\" / repovow tui");
    println!("Moat: repovow policy init  (signed goals — hooks block tampered policy)");
    Ok(())
}

fn cmd_agents(cmd: AgentsCmd) -> Result<()> {
    match cmd {
        AgentsCmd::Install => {
            let (claude, codex) = install_global_hooks()?;
            println!("Claude Code router: {}", claude.display());
            println!("Codex router: {}", codex.display());
            println!("RepoVow now bootstraps automatically when an agent opens a Git repository.");
            println!("Claude Code reloads this setting live.");
            println!("Codex: RepoVow-owned hooks are trusted; a session that predates installation may need one restart.");
        }
        AgentsCmd::Status => {
            for (agent, path, installed, active) in global_hooks_status()? {
                let state = if !installed {
                    "missing"
                } else if !active {
                    "installed, trust missing"
                } else {
                    "active"
                };
                println!("{}: {} ({})", agent, state, path.display());
            }
        }
    }
    Ok(())
}

fn cmd_goal_set(
    title: &str,
    accept: Vec<String>,
    constraint: Vec<String>,
    step: Option<String>,
) -> Result<()> {
    let form = GoalForm {
        title: title.to_string(),
        step: step.unwrap_or_default(),
        acceptance: accept,
        constraints: constraint,
    };
    save_goal(&form, None, "cli")?;
    println!("Goal set: {title}");
    Ok(())
}

fn cmd_goal_show() -> Result<()> {
    let state = load_state(None)?;
    match state.goal {
        Some(goal) => println!("{}", serde_json::to_string_pretty(&goal)?),
        None => println!("No active goal. Run: repovow goal set \"...\""),
    }
    Ok(())
}

fn cmd_progress(step: Option<String>, done: Option<String>, blocker: Option<String>) -> Result<()> {
    let mut state = load_state(None)?;
    if let Some(s) = step {
        state.progress.current_step = Some(s.clone());
        println!("Current step: {s}");
    }
    if let Some(d) = done {
        state.progress.completed.push(d.clone());
        println!("Marked done: {d}");
    }
    if let Some(b) = blocker {
        state.progress.blockers.push(b.clone());
        println!("Blocker: {b}");
    }
    save_state(&mut state, None)?;
    write_snapshot(None)?;
    sync_cloud_after_write()?;
    Ok(())
}

fn cmd_decide(text: &str) -> Result<()> {
    let mut state = load_state(None)?;
    state.decisions.push(Decision {
        at: utcnow(),
        text: text.to_string(),
    });
    save_state(&mut state, None)?;
    write_snapshot(None)?;
    sync_cloud_after_write()?;
    println!("Recorded decision: {text}");
    Ok(())
}

fn cmd_status() -> Result<()> {
    let root = find_project_root(None);
    let state = load_state(None)?;
    let goal = state
        .goal
        .as_ref()
        .map(|g| g.title.as_str())
        .unwrap_or("(none)");
    let step = state.progress.current_step.as_deref().unwrap_or("(none)");
    println!("Project: {}", root.display());
    println!("Goal: {goal}");
    println!("Step: {step}");
    println!(
        "Compactions: {} · Sessions: {}",
        state.compactions, state.sessions
    );
    println!(
        "Last agent: {}",
        state.last_agent.as_deref().unwrap_or("unknown")
    );
    println!("Snapshot: {}", root.join(".repovow/snapshot.md").display());
    Ok(())
}

fn cmd_snapshot(print: bool) -> Result<()> {
    if print {
        print!("{}", render_snapshot(None)?);
    } else {
        let path = write_snapshot(None)?;
        sync_cloud_after_write()?;
        println!("Wrote {}", path.display());
    }
    Ok(())
}

fn cmd_context(stats: bool, section: Option<&str>) -> Result<()> {
    if let Some(section) = section {
        let section: ContextSection = section.parse().map_err(anyhow::Error::msg)?;
        let text = render_context_section(None, section)?;
        print!("{text}");
        if stats {
            eprintln!(
                "Section {}: ~{} tokens",
                section.as_str(),
                repovow::context::estimate_tokens(&text)
            );
        }
        return Ok(());
    }
    let context = render_context(None)?;
    print!("{}", context.text);
    if stats {
        eprintln!(
            "Context: ~{} tokens; full snapshot: ~{}; saved: ~{} ({}%)",
            context.estimated_tokens,
            context.snapshot_estimated_tokens,
            context.saved_tokens(),
            context.savings_percent()
        );
    }
    Ok(())
}

fn sync_cloud_after_write() -> Result<()> {
    push_state(None)
}

fn cmd_cloud_link(url: &str, project: &str, key: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    if !cwd.join(".git").exists() && !cwd.join(".repovow").exists() {
        eprintln!(
            "Tip: run `cd your-project` before `repovow cloud link` so state stays in the repo."
        );
    }
    save_cloud_config(
        &CloudConfig {
            url: url.trim_end_matches('/').to_string(),
            project_id: project.to_string(),
            api_key: key.to_string(),
        },
        None,
    )?;
    let local_before = load_state(None)?;
    pull_state(None)?;
    let pulled = load_state(None)?;
    // New cloud projects start with `{}` — do not wipe an existing local goal.
    if local_before.goal.is_some() && pulled.goal.is_none() {
        let mut restored = local_before;
        save_state(&mut restored, None)?;
        write_snapshot(None)?;
        push_state(None)?;
        println!("Linked to RepoVow Cloud project {project}");
        println!("Uploaded local state (cloud project was empty).");
    } else {
        println!("Linked to RepoVow Cloud project {project}");
        println!("Pulled state from cloud.");
    }
    println!("URL: {}", url.trim_end_matches('/'));
    Ok(())
}

fn cmd_cloud_push() -> Result<()> {
    write_snapshot(None)?;
    push_state(None)?;
    println!("Pushed to RepoVow Cloud");
    Ok(())
}

fn cmd_cloud_pull() -> Result<()> {
    pull_state(None)?;
    println!("Pulled from RepoVow Cloud");
    Ok(())
}
