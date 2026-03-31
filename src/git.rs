use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context, Result};

pub const CONFIG_FILE: &str = "ssmver.toml";
pub const SSMVER_DIR: &str = ".ssmver";

pub fn git_repo_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("failed to run `git rev-parse --show-toplevel`")?;

    if !output.status.success() {
        bail!("ssmver must be run inside a git repository");
    }

    let root = String::from_utf8(output.stdout).context("git output was not valid UTF-8")?;
    Ok(PathBuf::from(root.trim()))
}

pub fn project_root_with_config() -> Result<PathBuf> {
    let cwd = env::current_dir().context("failed to read current directory")?;

    for ancestor in cwd.ancestors() {
        if ancestor.join(CONFIG_FILE).is_file() {
            return Ok(ancestor.to_path_buf());
        }
    }

    if let Ok(repo_root) = git_repo_root() {
        if repo_root.join(CONFIG_FILE).is_file() {
            return Ok(repo_root);
        }
    }

    bail!("Could not find ssmver.toml. Run `ssmver init` first.")
}

pub fn ensure_gitignore_has_ssmver(repo_root: &Path) -> Result<bool> {
    let gitignore_path = repo_root.join(".gitignore");
    let existing = match fs::read_to_string(&gitignore_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", gitignore_path.display()))
        }
    };

    if existing
        .lines()
        .any(|line| matches_ssmver_ignore(line.trim()))
    {
        return Ok(false);
    }

    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(".ssmver/\n");

    fs::write(&gitignore_path, updated)
        .with_context(|| format!("failed to write {}", gitignore_path.display()))?;
    Ok(true)
}

pub fn remove_ssmver_from_gitignore(repo_root: &Path) -> Result<bool> {
    let gitignore_path = repo_root.join(".gitignore");
    let existing = match fs::read_to_string(&gitignore_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", gitignore_path.display()))
        }
    };

    let lines: Vec<&str> = existing
        .lines()
        .filter(|line| !matches_ssmver_ignore(line.trim()))
        .collect();

    if lines.len() == existing.lines().count() {
        return Ok(false);
    }

    let mut rewritten = lines.join("\n");
    if !rewritten.is_empty() {
        rewritten.push('\n');
    }

    fs::write(&gitignore_path, rewritten)
        .with_context(|| format!("failed to write {}", gitignore_path.display()))?;
    Ok(true)
}

pub fn set_hooks_path(repo_root: &Path) -> Result<()> {
    run_git(repo_root, ["config", "core.hooksPath", ".ssmver/hooks"])?;
    Ok(())
}

pub fn unset_hooks_path(repo_root: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["config", "--unset", "core.hooksPath"])
        .current_dir(repo_root)
        .output()
        .context("failed to run `git config --unset core.hooksPath`")?;

    if output.status.success() || output.status.code() == Some(5) {
        return Ok(());
    }

    bail!(
        "failed to unset core.hooksPath: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

pub fn run_git<I, S>(repo_root: &Path, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args_vec = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect::<Vec<_>>();
    let output = Command::new("git")
        .args(&args_vec)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to run `git {}`", args_vec.join(" ")))?;

    if !output.status.success() {
        bail!("git {} failed: {}", args_vec.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(())
}

pub fn try_git_output<I, S>(repo_root: &Path, args: I) -> Result<Option<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args_vec = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect::<Vec<_>>();
    let output = Command::new("git")
        .args(&args_vec)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to run `git {}`", args_vec.join(" ")))?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(Some(
        String::from_utf8(output.stdout).context("git output was not valid UTF-8")?,
    ))
}

pub fn find_main_ref(repo_root: &Path) -> Result<Option<String>> {
    for reference in [
        "refs/heads/main",
        "refs/heads/master",
        "refs/remotes/origin/main",
        "refs/remotes/origin/master",
    ] {
        let output = Command::new("git")
            .args(["show-ref", "--verify", "--quiet", reference])
            .current_dir(repo_root)
            .output()
            .with_context(|| format!("failed to run `git show-ref --verify {reference}`"))?;
        if output.status.success() {
            return Ok(Some(reference.to_string()));
        }
    }

    Ok(None)
}

pub fn merge_base(repo_root: &Path, reference: &str) -> Result<Option<String>> {
    try_git_output(repo_root, ["merge-base", "HEAD", reference]).map(|result| {
        result.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    })
}

pub fn log_subjects_since(repo_root: &Path, revision_range: &str) -> Result<Vec<String>> {
    match try_git_output(repo_root, ["log", "--format=%s", revision_range])? {
        Some(output) => Ok(output
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect()),
        None => Ok(Vec::new()),
    }
}

pub fn show_file_at_rev(repo_root: &Path, revision: &str, relative_path: &Path) -> Result<Option<String>> {
    let spec = format!("{revision}:{}", relative_path.to_string_lossy());
    try_git_output(repo_root, ["show", &spec])
}

fn matches_ssmver_ignore(line: &str) -> bool {
    matches!(line, ".ssmver" | ".ssmver/")
}
