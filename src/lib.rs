pub mod acceptance;
pub mod check;
pub mod cloud;
pub mod constraints;
pub mod context;
pub mod doctor;
pub mod goal_edit;
pub mod hooks;
pub mod install;
pub mod loop_breaker;
pub mod onboard;
pub mod paths;
pub mod policy;
pub mod server;
pub mod snapshot;
pub mod state;
pub mod tui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPOVOW_DIR: &str = ".repovow";

/// Read a RepoVow environment setting while honoring its pre-rename equivalent.
/// New settings always win, so deployments can migrate without a flag day.
pub fn env_var(name: &str) -> Result<String, std::env::VarError> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => {
            let Some(suffix) = name.strip_prefix("REPOVOW_") else {
                return std::env::var(name);
            };
            std::env::var(format!("KEEL_{suffix}"))
        }
        Err(error) => Err(error),
    }
}
