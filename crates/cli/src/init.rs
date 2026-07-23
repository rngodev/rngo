mod skills;

use std::error::Error;
use std::fs;
use std::path::Path;

use dialoguer::Confirm;

pub fn init(base: &Path) -> Result<(), Box<dyn Error>> {
    let rngo_dir = base.join(".rngo");
    fs::create_dir_all(&rngo_dir)?;

    let spec_path = rngo_dir.join("spec.yml");
    if spec_path.exists() {
        println!(".rngo is already set up.");
    } else {
        let name = project_name(base)?;
        fs::write(&spec_path, format!("key: {name}\nseed: 1\n"))?;
        println!("Set up .rngo.");
    }

    match ensure_gitignore(base, confirm_create_gitignore)? {
        GitignoreOutcome::Created => println!("Created .gitignore."),
        GitignoreOutcome::Updated => println!("Updated .gitignore."),
        GitignoreOutcome::AlreadyUpToDate => println!(".gitignore already up to date."),
        GitignoreOutcome::Skipped => {}
    }

    skills::offer_install(base);

    Ok(())
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

        init(base).unwrap();

        let spec = fs::read_to_string(base.join(".rngo/spec.yml")).unwrap();
        assert_eq!(spec, format!("key: {name}\nseed: 1\n"));
    }

    #[test]
    fn appends_to_existing_gitignore_without_duplicating() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        fs::write(base.join(".gitignore"), "target\n").unwrap();

        init(base).unwrap();
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

        init(base).unwrap();

        let spec = fs::read_to_string(base.join(".rngo/spec.yml")).unwrap();
        assert_eq!(spec, "seed: 1\n");
    }
}
