use std::error::Error;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use dialoguer::{Confirm, Input, Select};
use semver::Version;
use tempfile::TempDir;

use crate::ui;

const RELEASES_URL: &str = "https://api.github.com/repos/rngodev/agent/releases/latest";
const USER_AGENT: &str = "rngo-cli";
const VERSION_FILE: &str = ".version";

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
    let install = Confirm::with_theme(&ui::theme())
        .with_prompt("Would you like to install agent skills?")
        .default(true)
        .interact()?;

    if !install {
        return Ok(());
    }

    let dir = prompt_location(base)?;
    do_install(&dir)
}

/// Downloads the latest rngo agent skills and installs them into `path`,
/// replacing any previously installed `rngo-` skills there. Prompts for a
/// location (from a set of common presets, or a custom one) when `path`
/// isn't given.
pub fn install(base: &Path, path: Option<PathBuf>) -> Result<(), Box<dyn Error>> {
    let dir = match path {
        Some(path) => path,
        None => prompt_location(base)?,
    };

    do_install(&dir)
}

fn do_install(dir: &Path) -> Result<(), Box<dyn Error>> {
    let zipball_url = fetch_latest_zipball_url()?;
    let (_tmp, skills) = fetch_skills(&zipball_url)?;

    ui::outcome(format!("{}:", dir.display()));
    remove_stale_skills(dir, &skills)?;
    install_skills(dir, &skills)?;

    Ok(())
}

/// Asks where to install skills: a set of common local/global presets, or a
/// custom path.
fn prompt_location(base: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let home = home_dir()?;
    let presets: [(&str, PathBuf); 4] = [
        ("Standard Local", base.join(".agents").join("skills")),
        ("Claude Local", base.join(".claude").join("skills")),
        ("Standard Global", home.join(".agents").join("skills")),
        ("Claude Global", home.join(".claude").join("skills")),
    ];

    let mut items: Vec<String> = presets
        .iter()
        .map(|(label, path)| format!("{label} ({})", display_path(path)))
        .collect();
    items.push("Other".to_string());

    let choice = Select::with_theme(&ui::theme())
        .with_prompt("Where should skills be installed?")
        .items(&items)
        .default(0)
        .interact()?;

    match presets.get(choice) {
        Some((_, path)) => Ok(path.clone()),
        None => {
            let input: String = Input::with_theme(&ui::theme())
                .with_prompt("Path")
                .interact_text()?;
            Ok(expand_tilde(&input))
        }
    }
}

/// Expands a leading `~` in a user-entered path to the home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = home_dir() {
            return home.join(rest);
        }
    } else if path == "~"
        && let Ok(home) = home_dir()
    {
        return home;
    }
    PathBuf::from(path)
}

/// Renders `path` with the user's home directory abbreviated to `~`, for
/// display in prompts (e.g. `~/.claude` instead of `/Users/name/.claude`).
fn display_path(path: &Path) -> String {
    if let Ok(home) = home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

/// Removes any `rngo-`-prefixed skill directory that isn't in the latest
/// release, so a fresh install can't leave behind skills that were renamed
/// or removed upstream. Skills still present in `skills` are left in place
/// here; `install_skills` handles updating those in place.
fn remove_stale_skills(skills_dir: &Path, skills: &[Skill]) -> Result<(), Box<dyn Error>> {
    if !skills_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(skills_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let still_current = skills.iter().any(|(current, _)| current == &name);

        if entry.file_type()?.is_dir() && name.starts_with("rngo-") && !still_current {
            fs::remove_dir_all(entry.path())?;
        }
    }

    Ok(())
}

/// Installs each skill, printing its previous and new version. Skills whose
/// installed version already matches the latest release are left untouched.
fn install_skills(skills_dir: &Path, skills: &[Skill]) -> Result<(), Box<dyn Error>> {
    for (name, src) in skills {
        let dest = skills_dir.join(name);
        let previous = skill_version(&dest.join(VERSION_FILE));
        let latest = skill_version(&src.join(VERSION_FILE));

        if previous.is_some() && previous == latest {
            let version = latest.map_or("unknown".to_string(), |v| v.to_string());
            ui::outcome(format!("  {name}: up to date ({version})"));
            continue;
        }

        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        copy_dir(src, &dest)?;

        match (previous, latest) {
            (Some(old), Some(new)) => ui::outcome(format!("  {name}: {old} -> {new}")),
            (None, Some(new)) => ui::outcome(format!("  {name}: installed {new}")),
            (_, None) => ui::outcome(format!("  {name}: installed")),
        }
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
    fn display_path_abbreviates_home_dir_with_tilde() {
        let home = home_dir().unwrap();
        assert_eq!(display_path(&home.join(".claude")), "~/.claude");
    }

    #[test]
    fn display_path_returns_full_path_outside_home_dir() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude");
        assert_eq!(display_path(&path), path.display().to_string());
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
    fn expand_tilde_expands_leading_home_dir() {
        let home = home_dir().unwrap();
        assert_eq!(expand_tilde("~/foo/skills"), home.join("foo/skills"));
        assert_eq!(expand_tilde("~"), home);
    }

    #[test]
    fn expand_tilde_leaves_other_paths_unchanged() {
        assert_eq!(expand_tilde("./foo/skills"), PathBuf::from("./foo/skills"));
        assert_eq!(expand_tilde("/abs/skills"), PathBuf::from("/abs/skills"));
    }

    #[test]
    fn remove_stale_skills_removes_rngo_dirs_no_longer_in_latest_release() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(skills_dir.join("rngo-removed-skill")).unwrap();
        fs::create_dir_all(skills_dir.join("rngo-system-inference")).unwrap();
        fs::create_dir_all(skills_dir.join("custom-skill")).unwrap();

        let skills = vec![(
            "rngo-system-inference".to_string(),
            tmp.path().join("latest").join("rngo-system-inference"),
        )];

        remove_stale_skills(&skills_dir, &skills).unwrap();

        assert!(!skills_dir.join("rngo-removed-skill").exists());
        assert!(skills_dir.join("rngo-system-inference").exists());
        assert!(skills_dir.join("custom-skill").exists());
    }

    #[test]
    fn remove_stale_skills_no_op_when_dir_missing() {
        let tmp = TempDir::new().unwrap();
        remove_stale_skills(&tmp.path().join("does-not-exist"), &[]).unwrap();
    }

    #[test]
    fn install_skills_skips_skill_already_at_latest_version() {
        let src_root = TempDir::new().unwrap();
        let src = src_root.path().join("rngo-system-inference");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "new content").unwrap();
        fs::write(src.join(".version"), "1.2.0").unwrap();
        let skills = vec![("rngo-system-inference".to_string(), src)];

        let dest_root = TempDir::new().unwrap();
        let installed = dest_root.path().join("rngo-system-inference");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("SKILL.md"), "old content").unwrap();
        fs::write(installed.join(".version"), "1.2.0").unwrap();

        install_skills(dest_root.path(), &skills).unwrap();

        assert_eq!(
            fs::read_to_string(installed.join("SKILL.md")).unwrap(),
            "old content"
        );
    }

    #[test]
    fn install_skills_replaces_skill_with_older_version() {
        let src_root = TempDir::new().unwrap();
        let src = src_root.path().join("rngo-system-inference");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "new content").unwrap();
        fs::write(src.join(".version"), "1.3.0").unwrap();
        let skills = vec![("rngo-system-inference".to_string(), src)];

        let dest_root = TempDir::new().unwrap();
        let installed = dest_root.path().join("rngo-system-inference");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("SKILL.md"), "old content").unwrap();
        fs::write(installed.join(".version"), "1.2.0").unwrap();

        install_skills(dest_root.path(), &skills).unwrap();

        assert_eq!(
            fs::read_to_string(installed.join("SKILL.md")).unwrap(),
            "new content"
        );
        assert_eq!(
            skill_version(&installed.join(".version")),
            Some(Version::new(1, 3, 0))
        );
    }
}
