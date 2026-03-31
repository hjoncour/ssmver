use std::{env, fs, path::{Path, PathBuf}};

use anyhow::{Context, Result};

const TEMPLATES_DIR: &str = "templates/changelog";

const TEMPLATE_KEEPACHANGELOG: (&str, &str) = ("keepachangelog", r#"## [{{version}}] - {{date}}

### Added
{{#added}}
- {{.}}
{{/added}}

### Changed
{{#changed}}
- {{.}}
{{/changed}}

### Fixed
{{#fixed}}
- {{.}}
{{/fixed}}
"#);

const TEMPLATE_CONVENTIONAL: (&str, &str) = ("conventional", r#"## {{version}} ({{date}})

{{#commits}}
* {{prefix}}: {{message}}
{{/commits}}
"#);

const TEMPLATE_SIMPLE: (&str, &str) = ("simple", r#"## {{version}}

{{#commits}}
- {{message}}
{{/commits}}
"#);

const DEFAULT_TEMPLATES: [(&str, &str); 3] = [TEMPLATE_KEEPACHANGELOG, TEMPLATE_CONVENTIONAL, TEMPLATE_SIMPLE];

pub fn ssmver_home() -> Result<PathBuf> {
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).context("could not determine home directory")?;
    Ok(PathBuf::from(home).join(".ssmver"))
}

pub fn templates_dir() -> Result<PathBuf> {
    Ok(ssmver_home()?.join(TEMPLATES_DIR))
}

fn templates_dir_at(base: &Path) -> PathBuf {
    base.join(TEMPLATES_DIR)
}

pub fn ensure_default_templates() -> Result<Vec<String>> {
    ensure_default_templates_at(&ssmver_home()?)
}

fn ensure_default_templates_at(base: &Path) -> Result<Vec<String>> {
    let dir = templates_dir_at(base);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;

    let mut created = Vec::new();
    for (name, content) in &DEFAULT_TEMPLATES {
        let path = dir.join(name);
        if !path.exists() {
            fs::write(&path, content).with_context(|| format!("failed to write template {}", path.display()))?;
            created.push(name.to_string());
        }
    }

    Ok(created)
}

pub fn resolve_template(name: &str) -> Result<Option<String>> {
    resolve_template_at(&ssmver_home()?, name)
}

fn resolve_template_at(base: &Path, name: &str) -> Result<Option<String>> {
    if name == "none" {
        return Ok(None);
    }
    let path = templates_dir_at(base).join(name);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).with_context(|| format!("failed to read template {}", path.display()))?;
    Ok(Some(content))
}

pub fn list_templates() -> Result<Vec<String>> {
    list_templates_at(&ssmver_home()?)
}

fn list_templates_at(base: &Path) -> Result<Vec<String>> {
    let dir = templates_dir_at(base);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn ensure_default_templates_creates_files() {
        let temp = TempDir::new().unwrap();
        let created = ensure_default_templates_at(temp.path()).unwrap();
        assert_eq!(created.len(), 3);
        assert!(created.contains(&"keepachangelog".to_string()));
        assert!(created.contains(&"conventional".to_string()));
        assert!(created.contains(&"simple".to_string()));

        let dir = templates_dir_at(temp.path());
        assert!(dir.join("keepachangelog").exists());
        assert!(dir.join("conventional").exists());
        assert!(dir.join("simple").exists());
    }

    #[test]
    fn ensure_default_templates_skips_existing() {
        let temp = TempDir::new().unwrap();
        ensure_default_templates_at(temp.path()).unwrap();
        let created = ensure_default_templates_at(temp.path()).unwrap();
        assert!(created.is_empty());
    }

    #[test]
    fn resolve_template_returns_content() {
        let temp = TempDir::new().unwrap();
        ensure_default_templates_at(temp.path()).unwrap();
        let content = resolve_template_at(temp.path(), "keepachangelog").unwrap();
        assert!(content.is_some());
        assert!(content.unwrap().contains("{{version}}"));
    }

    #[test]
    fn resolve_template_none_returns_none() {
        let temp = TempDir::new().unwrap();
        let content = resolve_template_at(temp.path(), "none").unwrap();
        assert!(content.is_none());
    }

    #[test]
    fn resolve_template_missing_returns_none() {
        let temp = TempDir::new().unwrap();
        let content = resolve_template_at(temp.path(), "nonexistent").unwrap();
        assert!(content.is_none());
    }

    #[test]
    fn list_templates_returns_sorted_names() {
        let temp = TempDir::new().unwrap();
        ensure_default_templates_at(temp.path()).unwrap();
        let names = list_templates_at(temp.path()).unwrap();
        assert_eq!(names, vec!["conventional", "keepachangelog", "simple"]);
    }

    #[test]
    fn list_templates_empty_when_no_dir() {
        let temp = TempDir::new().unwrap();
        let names = list_templates_at(temp.path()).unwrap();
        assert!(names.is_empty());
    }
}
