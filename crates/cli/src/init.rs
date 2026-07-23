mod skills;

use std::error::Error;
use std::fs;
use std::path::Path;

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

    if ensure_gitignore(base)? {
        println!("Updated .gitignore.");
    } else {
        println!(".gitignore already up to date.");
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

/// Returns `true` if `.gitignore` was created or modified.
fn ensure_gitignore(base: &Path) -> Result<bool, Box<dyn Error>> {
    let path = base.join(".gitignore");
    let entry = ".rngo/runs";

    let contents = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };

    if contents.lines().any(|line| line.trim() == entry) {
        return Ok(false);
    }

    let mut updated = contents;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(entry);
    updated.push('\n');

    fs::write(&path, updated)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn creates_spec_and_gitignore() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
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

        let gitignore = fs::read_to_string(base.join(".gitignore")).unwrap();
        assert_eq!(gitignore, ".rngo/runs\n");
    }

    #[test]
    fn appends_to_existing_gitignore_without_duplicating() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        fs::write(base.join(".gitignore"), "target\n").unwrap();

        init(base).unwrap();
        let gitignore = fs::read_to_string(base.join(".gitignore")).unwrap();
        assert_eq!(gitignore, "target\n.rngo/runs\n");

        ensure_gitignore(base).unwrap();
        let gitignore = fs::read_to_string(base.join(".gitignore")).unwrap();
        assert_eq!(gitignore, "target\n.rngo/runs\n");
    }

    #[test]
    fn does_not_overwrite_existing_spec() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        fs::create_dir_all(base.join(".rngo")).unwrap();
        fs::write(base.join(".rngo/spec.yml"), "seed: 1\n").unwrap();

        init(base).unwrap();

        let spec = fs::read_to_string(base.join(".rngo/spec.yml")).unwrap();
        assert_eq!(spec, "seed: 1\n");
    }
}
