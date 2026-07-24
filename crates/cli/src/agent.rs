use std::error::Error;
use std::fs;
use std::path::Path;

/// The coding agent configured for a project, read from the `agent` field of
/// `.rngo/spec.yml`.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentConfig {
    /// A built-in agent, identified by its shorthand name (e.g. `claude-code`).
    Named(String),
    Custom {
        config: String,
        command: String,
    },
}

impl AgentConfig {
    pub fn claude_code() -> Self {
        AgentConfig::Named("claude-code".to_string())
    }

    pub fn cursor() -> Self {
        AgentConfig::Named("cursor".to_string())
    }

    pub fn codex() -> Self {
        AgentConfig::Named("codex".to_string())
    }

    /// The directory (relative to a project or home root) this agent's
    /// configuration - and thus its skills - lives under, e.g. `.claude`.
    pub fn config_dir(&self) -> Option<&str> {
        match self {
            AgentConfig::Named(name) => named_config_dir(name),
            AgentConfig::Custom { config, .. } => Some(config.as_str()),
        }
    }

    /// Renders this config as the `agent:` field of `.rngo/spec.yml`.
    pub fn to_yaml_field(&self) -> String {
        match self {
            AgentConfig::Named(name) => format!("agent: {name}\n"),
            AgentConfig::Custom { config, command } => {
                format!("agent:\n  config: {config}\n  command: {command}\n")
            }
        }
    }

    fn from_value(value: &serde_json::Value) -> Result<AgentConfig, Box<dyn Error>> {
        match value {
            serde_json::Value::String(name) => Ok(AgentConfig::Named(name.clone())),
            serde_json::Value::Object(map) => {
                let config = map
                    .get("config")
                    .and_then(|v| v.as_str())
                    .ok_or("agent.config must be a string")?
                    .to_string();
                let command = map
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or("agent.command must be a string")?
                    .to_string();
                Ok(AgentConfig::Custom { config, command })
            }
            _ => Err("agent must be a string, or an object with config and command".into()),
        }
    }
}

fn named_config_dir(name: &str) -> Option<&'static str> {
    match name {
        "claude-code" => Some(".claude"),
        "cursor" => Some(".cursor"),
        "codex" => Some(".codex"),
        _ => None,
    }
}

/// Reads the `agent` field out of `<base>/.rngo/spec.yml`, if the file and
/// field both exist.
pub fn load(base: &Path) -> Result<Option<AgentConfig>, Box<dyn Error>> {
    let spec_path = base.join(".rngo/spec.yml");
    if !spec_path.exists() {
        return Ok(None);
    }

    let value: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&spec_path)?)?;
    match value.get("agent") {
        Some(agent) => Ok(Some(AgentConfig::from_value(agent)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn loads_named_agent() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".rngo")).unwrap();
        fs::write(
            tmp.path().join(".rngo/spec.yml"),
            "key: test\nseed: 1\nagent: claude-code\n",
        )
        .unwrap();

        assert_eq!(load(tmp.path()).unwrap(), Some(AgentConfig::claude_code()));
    }

    #[test]
    fn loads_custom_agent() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".rngo")).unwrap();
        fs::write(
            tmp.path().join(".rngo/spec.yml"),
            "key: test\nseed: 1\nagent:\n  config: .agents\n  command: myagent\n",
        )
        .unwrap();

        assert_eq!(
            load(tmp.path()).unwrap(),
            Some(AgentConfig::Custom {
                config: ".agents".to_string(),
                command: "myagent".to_string()
            })
        );
    }

    #[test]
    fn returns_none_when_no_agent_field() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".rngo")).unwrap();
        fs::write(tmp.path().join(".rngo/spec.yml"), "key: test\nseed: 1\n").unwrap();

        assert_eq!(load(tmp.path()).unwrap(), None);
    }

    #[test]
    fn returns_none_when_no_spec_file() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(load(tmp.path()).unwrap(), None);
    }

    #[test]
    fn config_dir_for_named_agents() {
        assert_eq!(AgentConfig::claude_code().config_dir(), Some(".claude"));
        assert_eq!(AgentConfig::cursor().config_dir(), Some(".cursor"));
        assert_eq!(AgentConfig::codex().config_dir(), Some(".codex"));
    }

    #[test]
    fn config_dir_for_custom_agent() {
        let agent = AgentConfig::Custom {
            config: ".agents".to_string(),
            command: "myagent".to_string(),
        };
        assert_eq!(agent.config_dir(), Some(".agents"));
    }

    #[test]
    fn config_dir_none_for_unrecognized_named_agent() {
        let agent = AgentConfig::Named("some-future-agent".to_string());
        assert_eq!(agent.config_dir(), None);
    }

    #[test]
    fn to_yaml_field_for_named_agent() {
        assert_eq!(
            AgentConfig::claude_code().to_yaml_field(),
            "agent: claude-code\n"
        );
    }

    #[test]
    fn to_yaml_field_for_custom_agent() {
        let agent = AgentConfig::Custom {
            config: ".agents".to_string(),
            command: "myagent".to_string(),
        };
        assert_eq!(
            agent.to_yaml_field(),
            "agent:\n  config: .agents\n  command: myagent\n"
        );
    }
}
