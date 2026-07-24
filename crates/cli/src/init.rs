use std::error::Error;
use std::fs;
use std::path::Path;

use dialoguer::{Confirm, Input, Select};

use crate::agent::{self, AgentConfig};
use crate::skills;

pub fn init(base: &Path) -> Result<(), Box<dyn Error>> {
    let agent = init_project(base, prompt_agent_config)?;
    skills::offer_install(base, agent.as_ref());
    Ok(())
}

/// Sets up `.rngo` and `.gitignore`, without touching agent skills. Split
/// out from `init` so tests can exercise it without triggering a network
/// call and interactive prompt from `skills::offer_install`.
fn init_project(
    base: &Path,
    prompt_agent: impl FnOnce() -> Option<AgentConfig>,
) -> Result<Option<AgentConfig>, Box<dyn Error>> {
    let rngo_dir = base.join(".rngo");
    fs::create_dir_all(&rngo_dir)?;

    let spec_path = rngo_dir.join("spec.yml");
    let agent = if spec_path.exists() {
        println!(".rngo is already set up.");
        agent::load(base)?
    } else {
        let name = project_name(base)?;
        let agent = prompt_agent();
        fs::write(&spec_path, spec_yaml(&name, agent.as_ref()))?;
        println!("Set up .rngo.");
        agent
    };

    match ensure_gitignore(base, confirm_create_gitignore)? {
        GitignoreOutcome::Created => println!("Created .gitignore."),
        GitignoreOutcome::Updated => println!("Updated .gitignore."),
        GitignoreOutcome::AlreadyUpToDate => println!(".gitignore already up to date."),
        GitignoreOutcome::Skipped => {}
    }

    Ok(agent)
}

fn spec_yaml(name: &str, agent: Option<&AgentConfig>) -> String {
    let mut spec = format!("key: {name}\nseed: 1\n");
    if let Some(agent) = agent {
        spec.push_str(&agent.to_yaml_field());
    }
    spec
}

/// Asks whether to configure a coding agent and, if so, which one. Errors
/// (e.g. no TTY) are swallowed so `rngo init` still succeeds non-interactively.
fn prompt_agent_config() -> Option<AgentConfig> {
    try_prompt_agent_config().unwrap_or(None)
}

fn try_prompt_agent_config() -> Result<Option<AgentConfig>, Box<dyn Error>> {
    let configure = Confirm::new()
        .with_prompt("Would you like to configure a coding agent?")
        .default(true)
        .interact()?;

    if !configure {
        return Ok(None);
    }

    let options = ["Claude Code", "Cursor", "Codex", "Custom"];
    let choice = Select::new()
        .with_prompt("Which coding agent do you use?")
        .items(options)
        .default(0)
        .interact()?;

    let agent = match choice {
        0 => AgentConfig::claude_code(),
        1 => AgentConfig::cursor(),
        2 => AgentConfig::codex(),
        _ => {
            let config: String = Input::new().with_prompt("Config path").interact_text()?;
            let command: String = Input::new().with_prompt("Command").interact_text()?;
            AgentConfig::Custom { config, command }
        }
    };

    Ok(Some(agent))
}

fn project_name(base: &Path) -> Result<String, Box<dyn Error>> {
    base.canonicalize()?
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "could not determine project directory name".into())
}

fn confirm_create_gitignore() -> bool {
    Confirm::new()
        .with_prompt("No .gitignore found. Create one?")
        .default(true)
        .interact()
        .unwrap_or(false)
}

#[derive(Debug, PartialEq, Eq)]
enum GitignoreOutcome {
    Created,
    Updated,
    AlreadyUpToDate,
    Skipped,
}

fn ensure_gitignore(
    base: &Path,
    confirm_create: impl FnOnce() -> bool,
) -> Result<GitignoreOutcome, Box<dyn Error>> {
    let path = base.join(".gitignore");
    let entry = ".rngo/runs";

    if !path.exists() {
        if !confirm_create() {
            return Ok(GitignoreOutcome::Skipped);
        }
        fs::write(&path, format!("{entry}\n"))?;
        return Ok(GitignoreOutcome::Created);
    }

    let contents = fs::read_to_string(&path)?;
    if contents.lines().any(|line| line.trim() == entry) {
        return Ok(GitignoreOutcome::AlreadyUpToDate);
    }

    let mut updated = contents;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(entry);
    updated.push('\n');

    fs::write(&path, updated)?;
    Ok(GitignoreOutcome::Updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn creates_spec() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        fs::write(base.join(".gitignore"), "").unwrap();
        let name = base
            .canonicalize()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        init_project(base, || None).unwrap();

        let spec = fs::read_to_string(base.join(".rngo/spec.yml")).unwrap();
        assert_eq!(spec, format!("key: {name}\nseed: 1\n"));
    }

    #[test]
    fn appends_to_existing_gitignore_without_duplicating() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        fs::write(base.join(".gitignore"), "target\n").unwrap();

        init_project(base, || None).unwrap();
        let gitignore = fs::read_to_string(base.join(".gitignore")).unwrap();
        assert_eq!(gitignore, "target\n.rngo/runs\n");

        let outcome = ensure_gitignore(base, || panic!("should not prompt")).unwrap();
        assert_eq!(outcome, GitignoreOutcome::AlreadyUpToDate);
        let gitignore = fs::read_to_string(base.join(".gitignore")).unwrap();
        assert_eq!(gitignore, "target\n.rngo/runs\n");
    }

    #[test]
    fn creates_gitignore_when_confirmed() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        let outcome = ensure_gitignore(base, || true).unwrap();
        assert_eq!(outcome, GitignoreOutcome::Created);

        let gitignore = fs::read_to_string(base.join(".gitignore")).unwrap();
        assert_eq!(gitignore, ".rngo/runs\n");
    }

    #[test]
    fn skips_gitignore_when_declined() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        let outcome = ensure_gitignore(base, || false).unwrap();
        assert_eq!(outcome, GitignoreOutcome::Skipped);

        assert!(!base.join(".gitignore").exists());
    }

    #[test]
    fn does_not_overwrite_existing_spec() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        fs::create_dir_all(base.join(".rngo")).unwrap();
        fs::write(base.join(".rngo/spec.yml"), "seed: 1\n").unwrap();
        fs::write(base.join(".gitignore"), "").unwrap();

        init_project(base, || None).unwrap();

        let spec = fs::read_to_string(base.join(".rngo/spec.yml")).unwrap();
        assert_eq!(spec, "seed: 1\n");
    }

    #[test]
    fn writes_named_agent_to_spec() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        fs::write(base.join(".gitignore"), "").unwrap();

        let agent = init_project(base, || Some(AgentConfig::cursor())).unwrap();

        assert_eq!(agent, Some(AgentConfig::cursor()));
        let spec = fs::read_to_string(base.join(".rngo/spec.yml")).unwrap();
        assert!(spec.ends_with("agent: cursor\n"), "spec was: {spec:?}");
    }

    #[test]
    fn writes_custom_agent_to_spec() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        fs::write(base.join(".gitignore"), "").unwrap();

        init_project(base, || {
            Some(AgentConfig::Custom {
                config: ".agents".to_string(),
                command: "myagent".to_string(),
            })
        })
        .unwrap();

        let spec = fs::read_to_string(base.join(".rngo/spec.yml")).unwrap();
        assert!(
            spec.ends_with("agent:\n  config: .agents\n  command: myagent\n"),
            "spec was: {spec:?}"
        );
    }

    #[test]
    fn does_not_prompt_for_agent_when_spec_already_exists() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        fs::create_dir_all(base.join(".rngo")).unwrap();
        fs::write(
            base.join(".rngo/spec.yml"),
            "key: test\nseed: 1\nagent: codex\n",
        )
        .unwrap();
        fs::write(base.join(".gitignore"), "").unwrap();

        let agent = init_project(base, || panic!("should not prompt")).unwrap();

        assert_eq!(agent, Some(AgentConfig::codex()));
    }
}
