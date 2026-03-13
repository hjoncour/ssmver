mod config;
mod hooks;

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use crate::config::{BumpLevel, SsmverConfig};

const CONFIG_FILE: &str = "ssmver.toml";
const SSMVER_DIR: &str = ".ssmver";

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Install automatic semantic versioning into a git repository"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Init,
    Prefix {
        #[command(subcommand)]
        command: PrefixCommands,
    },
    Bump {
        level: BumpLevel,
    },
    Version,
    Config {
        key: String,
        value: Option<String>,
    },
    Uninstall,
}

#[derive(Debug, Subcommand)]
enum PrefixCommands {
    Add { name: String, level: BumpLevel },
    Remove { name: String },
    List,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Init) {
        Commands::Init => handle_init(),
        Commands::Prefix { command } => handle_prefix(command),
        Commands::Bump { level } => handle_bump(level),
        Commands::Version => handle_version(),
        Commands::Config { key, value } => handle_config(&key, value.as_deref()),
        Commands::Uninstall => handle_uninstall(),
    }
}

fn handle_init() -> Result<()> {
    let repo_root = git_repo_root()?;
    let install_root = repo_root.join(SSMVER_DIR);
    let hooks_dir = install_root.join("hooks");
    let scripts_dir = install_root.join("scripts");
    let config_path = repo_root.join(CONFIG_FILE);

    let mut summary = Vec::new();

    fs::create_dir_all(&hooks_dir)
        .with_context(|| format!("failed to create {}", hooks_dir.display()))?;
    fs::create_dir_all(&scripts_dir)
        .with_context(|| format!("failed to create {}", scripts_dir.display()))?;
    summary.push(format!("Ensured {}", install_root.display()));

    write_executable(
        &hooks_dir.join("prepare-commit-msg"),
        hooks::PREPARE_COMMIT_MSG,
    )?;
    write_executable(&hooks_dir.join("post-commit"), hooks::POST_COMMIT)?;
    write_executable(&scripts_dir.join("bump_version.sh"), hooks::BUMP_VERSION)?;
    summary.push("Generated hooks and scripts".to_string());

    set_hooks_path(&repo_root)?;
    summary.push("Set git core.hooksPath to .ssmver/hooks".to_string());

    if ensure_gitignore_has_ssmver(&repo_root)? {
        summary.push("Updated .gitignore with .ssmver/".to_string());
    } else {
        summary.push(".gitignore already ignored .ssmver/".to_string());
    }

    if config_path.exists() {
        summary.push("Preserved existing ssmver.toml".to_string());
    } else {
        SsmverConfig::default().save(&config_path)?;
        summary.push("Created default ssmver.toml".to_string());
    }

    for line in summary {
        println!("{line}");
    }

    Ok(())
}

fn handle_prefix(command: PrefixCommands) -> Result<()> {
    let root = project_root_with_config()?;
    let config_path = root.join(CONFIG_FILE);
    let mut config = SsmverConfig::load(&config_path)?;

    match command {
        PrefixCommands::Add { name, level } => {
            config.prefixes.insert(name.clone(), level);
            config.save(&config_path)?;
            println!("Added prefix: {name} -> {level}");
        }
        PrefixCommands::Remove { name } => {
            if config.prefixes.remove(&name).is_none() {
                bail!("Prefix not found: {name}");
            }
            config.save(&config_path)?;
            println!("Removed prefix: {name}");
        }
        PrefixCommands::List => {
            if config.prefixes.is_empty() {
                println!("No prefixes configured");
            } else {
                for (prefix, level) in config.prefixes {
                    println!("{prefix:<10} -> {level}");
                }
            }
        }
    }

    Ok(())
}

fn handle_bump(level: BumpLevel) -> Result<()> {
    let root = project_root_with_config()?;
    let config_path = root.join(CONFIG_FILE);
    let mut config = SsmverConfig::load(&config_path)?;
    let (before, after) = config.bump_version(level);
    config.save(&config_path)?;
    println!("{before} -> {after}");
    Ok(())
}

fn handle_version() -> Result<()> {
    let root = project_root_with_config()?;
    let config = SsmverConfig::load(&root.join(CONFIG_FILE))?;
    println!("{}", config.version);
    Ok(())
}

fn handle_config(key: &str, value: Option<&str>) -> Result<()> {
    let root = project_root_with_config()?;
    let config_path = root.join(CONFIG_FILE);
    let mut config = SsmverConfig::load(&config_path)?;

    match value {
        Some(raw_value) => {
            let message = config.set_config_value(key, raw_value)?;
            config.save(&config_path)?;
            println!("{message}");
        }
        None => {
            let current_value = config.get_config_value(key)?;
            println!("{current_value}");
        }
    }

    Ok(())
}

fn handle_uninstall() -> Result<()> {
    let repo_root = git_repo_root()?;
    let install_root = repo_root.join(SSMVER_DIR);
    let mut summary = Vec::new();

    if install_root.exists() {
        fs::remove_dir_all(&install_root)
            .with_context(|| format!("failed to remove {}", install_root.display()))?;
        summary.push("Removed .ssmver directory".to_string());
    } else {
        summary.push(".ssmver directory was already absent".to_string());
    }

    unset_hooks_path(&repo_root)?;
    summary.push("Unset git core.hooksPath".to_string());

    if remove_ssmver_from_gitignore(&repo_root)? {
        summary.push("Removed .ssmver/ from .gitignore".to_string());
    } else {
        summary.push(".gitignore did not contain .ssmver/".to_string());
    }

    summary.push("Left ssmver.toml untouched".to_string());

    for line in summary {
        println!("{line}");
    }

    Ok(())
}

fn project_root_with_config() -> Result<PathBuf> {
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

fn git_repo_root() -> Result<PathBuf> {
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

fn write_executable(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)
            .with_context(|| format!("failed to read metadata for {}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to chmod {}", path.display()))?;
    }

    Ok(())
}

fn ensure_gitignore_has_ssmver(repo_root: &Path) -> Result<bool> {
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

fn remove_ssmver_from_gitignore(repo_root: &Path) -> Result<bool> {
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

fn matches_ssmver_ignore(line: &str) -> bool {
    matches!(line, ".ssmver" | ".ssmver/")
}

fn set_hooks_path(repo_root: &Path) -> Result<()> {
    run_git(repo_root, ["config", "core.hooksPath", ".ssmver/hooks"])?;
    Ok(())
}

fn unset_hooks_path(repo_root: &Path) -> Result<()> {
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

fn run_git<const N: usize>(repo_root: &Path, args: [&str; N]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to run `git {}`", args.join(" ")))?;

    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(())
}
