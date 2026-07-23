use std::error::Error;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use dialoguer::{Confirm, Select};
use semver::Version;
use tempfile::TempDir;

const RELEASES_URL: &str = "https://api.github.com/repos/rngodev/agent/releases/latest";
const USER_AGENT: &str = "rngo-cli";
const VERSION_FILE: &str = ".version";

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

/// A skill directory found inside the extracted release archive, keyed by
/// its directory name (e.g. `rngo-system-inference`).
type Skill = (String, PathBuf);

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

    let zipball_url = fetch_latest_zipball_url()?;
    let (_tmp, skills) = fetch_skills(&zipball_url)?;

    let present: Vec<_> = global_locations
        .iter()
        .filter(|(_, dir)| {
            skills
                .iter()
                .any(|(name, _)| dir.join(name).join(VERSION_FILE).exists())
        })
        .collect();

    if present.is_empty() {
        return offer_fresh_install(base, &skills);
    }

    let outdated: Vec<_> = present
        .into_iter()
        .filter(|(_, dir)| location_outdated(dir, &skills).unwrap_or(true))
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
            install_skills(dir, &skills)?;
        }
        println!("Updated rngo agent skills.");
    }

    Ok(())
}

fn offer_fresh_install(base: &Path, skills: &[Skill]) -> Result<(), Box<dyn Error>> {
    let install = Confirm::new()
        .with_prompt("Install rngo agent skills?")
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

    let root = if scope == 0 {
        base.to_path_buf()
    } else {
        home_dir()?
    };

    let agent_dir = prompt_agent_dir()?;
    let dir = root.join(agent_dir.label()).join("skills");

    install_skills(&dir, skills)?;

    println!("Installed rngo agent skills.");
    Ok(())
}

/// Downloads the latest rngo agent skills and installs them, replacing any
/// previously installed `rngo-` skills in the target directory(ies).
///
/// When `dir` is `None`, installs into every agent directory (`.claude`,
/// `.agents`) already present under the install root, prompting for one if
/// neither is present.
pub fn install(base: &Path, global: bool, dir: Option<&str>) -> Result<(), Box<dyn Error>> {
    let root = if global {
        home_dir()?
    } else {
        base.to_path_buf()
    };
    let targets = resolve_targets(&root, dir, prompt_agent_dir)?;

    let zipball_url = fetch_latest_zipball_url()?;
    let (_tmp, skills) = fetch_skills(&zipball_url)?;

    for skills_dir in &targets {
        remove_rngo_skills(skills_dir)?;
        install_skills(skills_dir, &skills)?;
        println!("Installed rngo agent skills to {}.", skills_dir.display());
    }

    Ok(())
}

fn resolve_targets(
    root: &Path,
    dir: Option<&str>,
    prompt_agent_dir: impl FnOnce() -> Result<AgentDir, Box<dyn Error>>,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    if let Some(dir) = dir {
        return Ok(vec![root.join(dir).join("skills")]);
    }

    let present: Vec<PathBuf> = [AgentDir::Claude, AgentDir::Agents]
        .into_iter()
        .filter(|d| root.join(d.label()).exists())
        .map(|d| root.join(d.label()).join("skills"))
        .collect();

    if !present.is_empty() {
        return Ok(present);
    }

    let chosen = prompt_agent_dir()?;
    Ok(vec![root.join(chosen.label()).join("skills")])
}

fn prompt_agent_dir() -> Result<AgentDir, Box<dyn Error>> {
    let choice = Select::new()
        .with_prompt("Where should skills be installed?")
        .items([".claude", ".agents"])
        .default(0)
        .interact()?;

    Ok(if choice == 0 {
        AgentDir::Claude
    } else {
        AgentDir::Agents
    })
}

/// Removes any existing `rngo-`-prefixed skill directories so a fresh
/// install can't leave behind skills that were renamed or removed upstream.
fn remove_rngo_skills(skills_dir: &Path) -> Result<(), Box<dyn Error>> {
    if !skills_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(skills_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.file_name().to_string_lossy().starts_with("rngo-") {
            fs::remove_dir_all(entry.path())?;
        }
    }

    Ok(())
}

fn install_skills(skills_dir: &Path, skills: &[Skill]) -> Result<(), Box<dyn Error>> {
    for (name, src) in skills {
        let dest = skills_dir.join(name);
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        copy_dir(src, &dest)?;
    }

    Ok(())
}

fn copy_dir(src: &Path, dest: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(dest)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let dest_path = dest.join(entry.file_name());

        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), &dest_path)?;
        }
    }

    Ok(())
}

fn location_outdated(dir: &Path, skills: &[Skill]) -> Result<bool, Box<dyn Error>> {
    for (name, src) in skills {
        let latest_version = skill_version(&src.join(VERSION_FILE))
            .ok_or_else(|| format!("skill \"{name}\" is missing a {VERSION_FILE} file"))?;

        match skill_version(&dir.join(name).join(VERSION_FILE)) {
            None => return Ok(true),
            Some(v) if v < latest_version => return Ok(true),
            Some(_) => {}
        }
    }

    Ok(false)
}

fn skill_version(path: &Path) -> Option<Version> {
    let content = fs::read_to_string(path).ok()?;
    Version::parse(content.trim()).ok()
}

fn list_skills(skills_root: &Path) -> Result<Vec<Skill>, Box<dyn Error>> {
    if !skills_root.exists() {
        return Ok(Vec::new());
    }

    let mut skills = Vec::new();
    for entry in fs::read_dir(skills_root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            skills.push((name, entry.path()));
        }
    }
    skills.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(skills)
}

fn fetch_skills(zipball_url: &str) -> Result<(TempDir, Vec<Skill>), Box<dyn Error>> {
    let zip_bytes = download(zipball_url)?;
    let extracted = extract_skills(&zip_bytes)?;
    let skills = list_skills(&extracted.path().join("skills"))?;

    if skills.is_empty() {
        return Err("release archive does not contain a skills directory".into());
    }

    Ok((extracted, skills))
}

fn extract_skills(zip_bytes: &[u8]) -> Result<TempDir, Box<dyn Error>> {
    let tmp = TempDir::new()?;
    let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes))?;
    archive.extract_unwrapped_root_dir(tmp.path(), zip::read::root_dir_common_filter)?;
    Ok(tmp)
}

fn fetch_latest_zipball_url() -> Result<String, Box<dyn Error>> {
    let json: serde_json::Value = ureq::get(RELEASES_URL)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_json()?;

    json["zipball_url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "latest release is missing a zipball_url".into())
}

fn download(url: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let bytes = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()?
        .body_mut()
        .with_config()
        .limit(50 * 1024 * 1024)
        .read_to_vec()?;
    Ok(bytes)
}

fn home_dir() -> Result<PathBuf, Box<dyn Error>> {
    dirs::home_dir().ok_or_else(|| "could not determine home directory".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn build_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            let options = SimpleFileOptions::default();
            for (path, content) in entries {
                writer.start_file(*path, options).unwrap();
                writer.write_all(content.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        buf
    }

    #[test]
    fn extracts_skills_dir_stripping_wrapper() {
        let zip_bytes = build_zip(&[
            (
                "agent-abc123/skills/rngo-system-inference/SKILL.md",
                "content",
            ),
            (
                "agent-abc123/skills/rngo-system-inference/.version",
                "0.2.0",
            ),
            (
                "agent-abc123/skills/rngo-effect-inference/SKILL.md",
                "content",
            ),
            (
                "agent-abc123/skills/rngo-effect-inference/.version",
                "0.2.0",
            ),
            ("agent-abc123/VERSION", "0.2.0"),
        ]);

        let extracted = extract_skills(&zip_bytes).unwrap();
        let skills = list_skills(&extracted.path().join("skills")).unwrap();

        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].0, "rngo-effect-inference");
        assert_eq!(skills[1].0, "rngo-system-inference");
        assert_eq!(
            skill_version(&skills[1].1.join(".version")),
            Some(Version::new(0, 2, 0))
        );
    }

    #[test]
    fn copies_skill_directory_recursively() {
        let src_root = TempDir::new().unwrap();
        let src = src_root.path().join("rngo-system-inference");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("SKILL.md"), "content").unwrap();
        fs::write(src.join(".version"), "0.2.0").unwrap();
        fs::write(src.join("nested").join("extra.md"), "extra").unwrap();

        let dest_root = TempDir::new().unwrap();
        let dest = dest_root.path().join("rngo-system-inference");

        copy_dir(&src, &dest).unwrap();

        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            "content"
        );
        assert_eq!(fs::read_to_string(dest.join(".version")).unwrap(), "0.2.0");
        assert_eq!(
            fs::read_to_string(dest.join("nested").join("extra.md")).unwrap(),
            "extra"
        );
    }

    #[test]
    fn install_skills_copies_version_file() {
        let src_root = TempDir::new().unwrap();
        let src = src_root.path().join("rngo-system-inference");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "content").unwrap();
        fs::write(src.join(".version"), "1.2.0").unwrap();
        let skills = vec![("rngo-system-inference".to_string(), src)];

        let dest_root = TempDir::new().unwrap();
        install_skills(dest_root.path(), &skills).unwrap();

        let installed = dest_root.path().join("rngo-system-inference");
        assert!(installed.join("SKILL.md").exists());
        assert_eq!(
            skill_version(&installed.join(".version")),
            Some(Version::new(1, 2, 0))
        );
    }

    #[test]
    fn skill_version_missing_when_no_file() {
        let dir = TempDir::new().unwrap();
        assert_eq!(skill_version(&dir.path().join(".version")), None);
    }

    #[test]
    fn skill_version_none_for_garbage_content() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".version");
        fs::write(&path, "not-a-version").unwrap();
        assert_eq!(skill_version(&path), None);
    }

    #[test]
    fn location_outdated_when_missing_or_behind() {
        let latest_root = TempDir::new().unwrap();
        let latest_skill = latest_root.path().join("rngo-system-inference");
        fs::create_dir_all(&latest_skill).unwrap();
        fs::write(latest_skill.join(".version"), "0.2.0").unwrap();
        let skills = vec![("rngo-system-inference".to_string(), latest_skill.clone())];

        let empty_dir = TempDir::new().unwrap();
        assert!(location_outdated(empty_dir.path(), &skills).unwrap());

        let behind_dir = TempDir::new().unwrap();
        let installed = behind_dir.path().join("rngo-system-inference");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join(".version"), "0.1.0").unwrap();
        assert!(location_outdated(behind_dir.path(), &skills).unwrap());

        let current_dir = TempDir::new().unwrap();
        let installed = current_dir.path().join("rngo-system-inference");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join(".version"), "0.2.0").unwrap();
        assert!(!location_outdated(current_dir.path(), &skills).unwrap());
    }

    #[test]
    fn resolve_targets_uses_explicit_dir_without_prompting() {
        let tmp = TempDir::new().unwrap();
        let targets =
            resolve_targets(tmp.path(), Some(".agents"), || panic!("should not prompt")).unwrap();

        assert_eq!(targets, vec![tmp.path().join(".agents").join("skills")]);
    }

    #[test]
    fn resolve_targets_uses_present_agent_dir_without_prompting() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".claude")).unwrap();

        let targets = resolve_targets(tmp.path(), None, || panic!("should not prompt")).unwrap();

        assert_eq!(targets, vec![tmp.path().join(".claude").join("skills")]);
    }

    #[test]
    fn resolve_targets_uses_all_present_agent_dirs() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        fs::create_dir_all(tmp.path().join(".agents")).unwrap();

        let mut targets = resolve_targets(tmp.path(), None, || panic!("should not prompt"))
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>();
        targets.sort();

        let mut expected = vec![
            tmp.path().join(".claude").join("skills"),
            tmp.path().join(".agents").join("skills"),
        ];
        expected.sort();

        assert_eq!(targets, expected);
    }

    #[test]
    fn resolve_targets_prompts_when_neither_agent_dir_present() {
        let tmp = TempDir::new().unwrap();

        let targets = resolve_targets(tmp.path(), None, || Ok(AgentDir::Agents)).unwrap();

        assert_eq!(targets, vec![tmp.path().join(".agents").join("skills")]);
    }

    #[test]
    fn remove_rngo_skills_only_removes_rngo_prefixed_dirs() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(skills_dir.join("rngo-system-inference")).unwrap();
        fs::create_dir_all(skills_dir.join("custom-skill")).unwrap();

        remove_rngo_skills(&skills_dir).unwrap();

        assert!(!skills_dir.join("rngo-system-inference").exists());
        assert!(skills_dir.join("custom-skill").exists());
    }

    #[test]
    fn remove_rngo_skills_no_op_when_dir_missing() {
        let tmp = TempDir::new().unwrap();
        remove_rngo_skills(&tmp.path().join("does-not-exist")).unwrap();
    }
}
