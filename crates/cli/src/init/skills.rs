use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use dialoguer::{Confirm, Select};

const BASE_URL: &str = "https://rngo.dev/llm/skills/latest";

struct SkillDef {
    key: &'static str,
    name: &'static str,
    file: &'static str,
}

const SKILLS: [SkillDef; 3] = [
    SkillDef {
        key: "systems",
        name: "rngo-systems",
        file: "systems.md",
    },
    SkillDef {
        key: "effects",
        name: "rngo-effects",
        file: "effects.md",
    },
    SkillDef {
        key: "schema",
        name: "rngo-schema",
        file: "schema.md",
    },
];

#[derive(Clone, Copy)]
enum AgentDir {
    Claude,
    Agents,
}

impl AgentDir {
    fn label(self) -> &'static str {
        match self {
            AgentDir::Claude => ".claude",
            AgentDir::Agents => ".agents",
        }
    }
}

/// Offers to install rngo agent skills, printing a warning instead of
/// failing `rngo init` if anything (network, prompts) goes wrong.
pub fn offer_install(base: &Path) {
    if let Err(e) = try_offer_install(base) {
        eprintln!("warning: could not check rngo agent skills: {e}");
    }
}

fn try_offer_install(base: &Path) -> Result<(), Box<dyn Error>> {
    let home = home_dir()?;
    let global_locations = [
        (AgentDir::Claude, home.join(".claude").join("skills")),
        (AgentDir::Agents, home.join(".agents").join("skills")),
    ];

    let present: Vec<_> = global_locations
        .iter()
        .filter(|(_, dir)| {
            SKILLS
                .iter()
                .any(|s| dir.join(s.name).join("SKILL.md").exists())
        })
        .collect();

    if present.is_empty() {
        return offer_fresh_install(base);
    }

    let latest = fetch_versions()?;

    let outdated: Vec<_> = present
        .into_iter()
        .filter(|(_, dir)| location_outdated(dir, &latest).unwrap_or(true))
        .collect();

    if outdated.is_empty() {
        println!("rngo agent skills are already installed and up to date.");
        return Ok(());
    }

    let list = outdated
        .iter()
        .map(|(agent, dir)| format!("  {} ({})", agent.label(), dir.display()))
        .collect::<Vec<_>>()
        .join("\n");

    let update = Confirm::new()
        .with_prompt(format!(
            "Your global rngo agent skills are out of date:\n{list}\nUpdate them now?"
        ))
        .default(true)
        .interact()?;

    if update {
        for (_, dir) in &outdated {
            install_skills(dir, &latest)?;
        }
        println!("Updated rngo agent skills.");
    }

    Ok(())
}

fn offer_fresh_install(base: &Path) -> Result<(), Box<dyn Error>> {
    let install = Confirm::new()
        .with_prompt("Install rngo agent skills (rngo-systems, rngo-effects, rngo-schema)?")
        .default(true)
        .interact()?;

    if !install {
        return Ok(());
    }

    let scope = Select::new()
        .with_prompt("Install skills locally (this project) or globally (all projects)?")
        .items(["Local", "Global"])
        .default(0)
        .interact()?;

    let agent_choice = Select::new()
        .with_prompt("Where should skills be installed?")
        .items([".claude", ".agents", "both"])
        .default(2)
        .interact()?;

    let root = if scope == 0 {
        base.to_path_buf()
    } else {
        home_dir()?
    };

    let mut dirs = Vec::new();
    if agent_choice == 0 || agent_choice == 2 {
        dirs.push(root.join(".claude").join("skills"));
    }
    if agent_choice == 1 || agent_choice == 2 {
        dirs.push(root.join(".agents").join("skills"));
    }

    let latest = fetch_versions()?;
    for dir in &dirs {
        install_skills(dir, &latest)?;
    }

    println!("Installed rngo agent skills.");
    Ok(())
}

fn install_skills(skills_dir: &Path, latest: &HashMap<String, u64>) -> Result<(), Box<dyn Error>> {
    for skill in &SKILLS {
        let version = *latest
            .get(skill.key)
            .ok_or_else(|| format!("no version found for skill \"{}\"", skill.key))?;
        let content = fetch_skill_content(skill.file)?;
        let content = inject_version(&content, version)?;

        let skill_dir = skills_dir.join(skill.name);
        fs::create_dir_all(&skill_dir)?;
        fs::write(skill_dir.join("SKILL.md"), content)?;
    }

    Ok(())
}

fn location_outdated(dir: &Path, latest: &HashMap<String, u64>) -> Result<bool, Box<dyn Error>> {
    for skill in &SKILLS {
        let path = dir.join(skill.name).join("SKILL.md");
        let latest_version = *latest
            .get(skill.key)
            .ok_or_else(|| format!("no version found for skill \"{}\"", skill.key))?;

        match installed_version(&path)? {
            None => return Ok(true),
            Some(v) if v < latest_version => return Ok(true),
            Some(_) => {}
        }
    }

    Ok(false)
}

fn installed_version(path: &Path) -> Result<Option<u64>, Box<dyn Error>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)?;
    let frontmatter = match parse_frontmatter(&content) {
        Some(fm) => fm,
        None => return Ok(None),
    };

    Ok(frontmatter.get("version").and_then(|v| v.as_u64()))
}

fn parse_frontmatter(content: &str) -> Option<serde_yaml::Mapping> {
    let rest = content.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    serde_yaml::from_str(frontmatter).ok()
}

fn inject_version(content: &str, version: u64) -> Result<String, Box<dyn Error>> {
    let rest = content
        .strip_prefix("---")
        .ok_or("skill content is missing frontmatter")?;
    let end = rest
        .find("\n---")
        .ok_or("skill content has an unterminated frontmatter block")?;
    let frontmatter = &rest[..end];
    let body = &rest[end + 4..];

    let mut doc: serde_yaml::Mapping = serde_yaml::from_str(frontmatter)?;
    doc.insert(
        serde_yaml::Value::String("version".to_string()),
        serde_yaml::Value::Number(version.into()),
    );

    let new_frontmatter = serde_yaml::to_string(&doc)?;
    Ok(format!("---\n{new_frontmatter}---{body}"))
}

fn fetch_versions() -> Result<HashMap<String, u64>, Box<dyn Error>> {
    let url = format!("{BASE_URL}/versions.json");
    let versions = ureq::get(&url).call()?.body_mut().read_json()?;
    Ok(versions)
}

fn fetch_skill_content(file: &str) -> Result<String, Box<dyn Error>> {
    let url = format!("{BASE_URL}/{file}");
    let content = ureq::get(&url).call()?.body_mut().read_to_string()?;
    Ok(content)
}

fn home_dir() -> Result<PathBuf, Box<dyn Error>> {
    dirs::home_dir().ok_or_else(|| "could not determine home directory".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_version_into_frontmatter() {
        let content = "---\nname: rngo-systems\ndescription: does things\n---\n\nBody text\n";
        let updated = inject_version(content, 3).unwrap();

        let frontmatter = parse_frontmatter(&updated).unwrap();
        assert_eq!(frontmatter.get("version").and_then(|v| v.as_u64()), Some(3));
        assert_eq!(
            frontmatter.get("name").and_then(|v| v.as_str()),
            Some("rngo-systems")
        );
        assert!(updated.ends_with("Body text\n"));
    }

    #[test]
    fn injects_version_replacing_existing() {
        let content = "---\nname: rngo-systems\nversion: 1\n---\n\nBody\n";
        let updated = inject_version(content, 2).unwrap();

        let frontmatter = parse_frontmatter(&updated).unwrap();
        assert_eq!(frontmatter.get("version").and_then(|v| v.as_u64()), Some(2));
    }

    #[test]
    fn reads_installed_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("SKILL.md");
        fs::write(&path, "---\nname: rngo-systems\nversion: 3\n---\n\nBody\n").unwrap();

        assert_eq!(installed_version(&path).unwrap(), Some(3));
    }

    #[test]
    fn missing_file_has_no_installed_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("SKILL.md");

        assert_eq!(installed_version(&path).unwrap(), None);
    }
}
