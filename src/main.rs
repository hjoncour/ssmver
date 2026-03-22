mod config;
mod git;
mod hooks;
mod targets;
mod workflows;

use std::{
    env,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{
    config::{compute_next_version, BumpLevel, PromptMode, SsmverConfig},
    git::{
        ensure_gitignore_has_ssmver, find_main_ref, git_repo_root, log_subjects_since, merge_base,
        project_root_with_config, remove_ssmver_from_gitignore, run_git, set_hooks_path,
        show_file_at_rev, unset_hooks_path, CONFIG_FILE, SSMVER_DIR,
    },
    targets::{
        apply_version_to_targets, blocking_targets, discover_targets, extract_commit_prefix,
        format_target_table, infer_seed_version, Ecosystem, TargetStatus, VersionTarget,
    },
    workflows::{WorkflowKind, GITHUB_PACKAGES_ECOSYSTEMS},
};

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
    Update,
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
    Targets {
        #[command(subcommand)]
        command: TargetsCommands,
    },
    Uninstall,
    /// Generate a GitHub Actions workflow to create GitHub Releases
    Release,
    /// Generate a GitHub Actions workflow to publish to GitHub Packages
    Package,
    /// Generate a GitHub Actions workflow to publish to npm
    Npm,
    /// Generate a GitHub Actions workflow to publish to crates.io
    Crates,
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        command: HookCommands,
    },
}

#[derive(Debug, Subcommand)]
enum PrefixCommands {
    Add { name: String, level: BumpLevel },
    Remove { name: String },
    List,
}

#[derive(Debug, Subcommand)]
enum TargetsCommands {
    List(TargetListArgs),
}

#[derive(Debug, Args)]
struct TargetListArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum HookCommands {
    PrepareCommitMsg {
        message_file: PathBuf,
        source: Option<String>,
        commit_sha: Option<String>,
    },
    PostCommit,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingSync {
    files: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Init) {
        Commands::Init => handle_init(),
        Commands::Update => handle_update(),
        Commands::Prefix { command } => handle_prefix(command),
        Commands::Bump { level } => handle_bump(level),
        Commands::Version => handle_version(),
        Commands::Config { key, value } => handle_config(&key, value.as_deref()),
        Commands::Targets { command } => handle_targets(command),
        Commands::Uninstall => handle_uninstall(),
        Commands::Release => handle_workflow(WorkflowKind::Release),
        Commands::Package => handle_workflow(WorkflowKind::Package),
        Commands::Npm => handle_workflow(WorkflowKind::Npm),
        Commands::Crates => handle_workflow(WorkflowKind::Crates),
        Commands::Hook { command } => handle_hook(command),
    }
}

fn handle_init() -> Result<()> {
    let repo_root = git_repo_root()?;
    let config_path = repo_root.join(CONFIG_FILE);

    let mut summary = install_or_refresh_repo(&repo_root)?;

    if config_path.exists() {
        summary.push("Preserved existing ssmver.toml".to_string());
    } else {
        let detected_targets = discover_targets(&repo_root, &SsmverConfig::default().sync)?;
        ensure_syncable_targets(&detected_targets)?;
        let initial_version =
            infer_seed_version(&detected_targets)?.unwrap_or_else(|| Version::new(0, 1, 0));
        let mut config = SsmverConfig::default();
        config.version = initial_version.clone();
        config.save(&config_path)?;
        summary.push(format!(
            "Created ssmver.toml at version {}",
            initial_version
        ));
    }

    for line in summary {
        println!("{line}");
    }

    Ok(())
}

fn handle_update() -> Result<()> {
    let repo_root = project_root_with_config()?;
    let config_path = repo_root.join(CONFIG_FILE);
    let mut config = SsmverConfig::load(&config_path)?;
    let current_version = config.version.clone();

    let mut summary = install_or_refresh_repo(&repo_root)?;
    let changed_files = sync_project_version(
        &repo_root,
        &config_path,
        &mut config,
        current_version,
        false,
    )?;

    if changed_files.is_empty() {
        summary.push("Versioned files were already in sync".to_string());
    } else {
        summary.push(format!(
            "Re-synced {} file(s) to version {}",
            changed_files.len(),
            config.version
        ));
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
    let repo_root = project_root_with_config()?;
    let config_path = repo_root.join(CONFIG_FILE);
    let mut config = SsmverConfig::load(&config_path)?;
    let before = config.version.clone();
    let after = compute_next_version(&before, level);
    sync_project_version(&repo_root, &config_path, &mut config, after.clone(), false)?;
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
    let repo_root = project_root_with_config()?;
    let config_path = repo_root.join(CONFIG_FILE);
    let mut config = SsmverConfig::load(&config_path)?;

    match value {
        Some(raw_value) => {
            if key == "version" || key == "settings.version" {
                let new_version: Version = raw_value.parse()?;
                sync_project_version(
                    &repo_root,
                    &config_path,
                    &mut config,
                    new_version.clone(),
                    false,
                )?;
                println!("Set version = \"{}\"", new_version);
            } else {
                let message = config.set_config_value(key, raw_value)?;
                config.save(&config_path)?;
                println!("{message}");
            }
        }
        None => {
            let current_value = config.get_config_value(key)?;
            println!("{current_value}");
        }
    }

    Ok(())
}

fn handle_targets(command: TargetsCommands) -> Result<()> {
    match command {
        TargetsCommands::List(args) => {
            let repo_root = git_repo_root()?;
            let config = repo_root
                .join(CONFIG_FILE)
                .is_file()
                .then(|| SsmverConfig::load(&repo_root.join(CONFIG_FILE)))
                .transpose()?
                .unwrap_or_default();
            let targets = discover_targets(&repo_root, &config.sync)?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&targets)?);
            } else if targets.is_empty() {
                println!("No supported version targets found");
            } else {
                println!("{}", format_target_table(&targets));
            }
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

fn handle_workflow(kind: WorkflowKind) -> Result<()> {
    let repo_root = project_root_with_config()?;
    let config_path = repo_root.join(CONFIG_FILE);
    let mut config = SsmverConfig::load(&config_path)?;
    let targets = discover_targets(&repo_root, &config.sync)?;

    let package_ecosystem = match kind {
        WorkflowKind::Release => None,
        WorkflowKind::Npm => {
            require_ecosystem(&targets, Ecosystem::Node, "npm")?;
            None
        }
        WorkflowKind::Crates => {
            require_ecosystem(&targets, Ecosystem::Cargo, "crates")?;
            None
        }
        WorkflowKind::Package => Some(resolve_package_ecosystem(&targets)?),
    };

    let main_ref = find_main_ref(&repo_root)?.unwrap_or_else(|| "refs/heads/main".to_string());
    let branch = workflows::branch_name_from_ref(&main_ref);

    if !config.release.enabled {
        config.release.enabled = true;
        config.save(&config_path)?;
        println!("Enabled [release] in ssmver.toml");
    }

    let yaml = match kind {
        WorkflowKind::Release  => workflows::release_workflow(&config.release, branch),
        WorkflowKind::Npm      => workflows::npm_workflow(&config.release, branch),
        WorkflowKind::Crates   => workflows::crates_workflow(&config.release, branch),
        WorkflowKind::Package  => workflows::package_workflow(&config.release, branch, package_ecosystem.unwrap()),
    };

    let workflows_dir = repo_root.join(".github").join("workflows");
    fs::create_dir_all(&workflows_dir).context("creating .github/workflows directory")?;

    let file_name = workflows::workflow_file_name(kind);
    let file_path = workflows_dir.join(file_name);
    let verb = if file_path.exists() {"Overwrote"} else {"Generated"};
    fs::write(&file_path, &yaml).with_context(|| format!("writing {}", file_path.display()))?;

    let relative = file_path.strip_prefix(&repo_root).unwrap_or(&file_path);
    println!("{verb} {}", relative.display());

    match kind {
        WorkflowKind::Npm => println!("Add NPM_TOKEN to your repository secrets"),
        WorkflowKind::Crates => println!("Add CARGO_REGISTRY_TOKEN to your repository secrets"),
        _ => {}
    }

    Ok(())
}

fn require_ecosystem(targets: &[VersionTarget], ecosystem: Ecosystem, command: &str) -> Result<()> {
    if !targets.iter().any(|t| t.ecosystem == ecosystem && t.status == TargetStatus::Managed) {
        bail!("No {ecosystem} targets found. `ssmver {command}` requires a {ecosystem} project.");
    }
    Ok(())
}

fn resolve_package_ecosystem(targets: &[VersionTarget]) -> Result<Ecosystem> {
    let supported: Vec<Ecosystem> = GITHUB_PACKAGES_ECOSYSTEMS.iter().copied().filter(|eco| targets.iter().any(|t| t.ecosystem == *eco && t.status == TargetStatus::Managed)).collect();
    if supported.is_empty() {
        let names: Vec<&str> = GITHUB_PACKAGES_ECOSYSTEMS.iter().map(|e| match e {
            Ecosystem::Node   => "Node",
            Ecosystem::Maven  => "Maven",
            Ecosystem::Gradle => "Gradle",
            Ecosystem::Dotnet => ".NET",
            Ecosystem::Ruby   => "Ruby",
            _                 => "unknown",
        }).collect();
        bail!("No ecosystem supporting GitHub Packages found. Supported: {}", names.join(", "));
    }
    if supported.len() > 1 {
        println!("Multiple ecosystems support GitHub Packages: {}. Using {}.", supported.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", "), supported[0]);
    }
    Ok(supported[0])
}

fn handle_hook(command: HookCommands) -> Result<()> {
    match command {
        HookCommands::PrepareCommitMsg {
            message_file,
            source,
            commit_sha: _,
        } => handle_hook_prepare_commit_msg(&message_file, source.as_deref()),
        HookCommands::PostCommit => handle_hook_post_commit(),
    }
}

fn handle_hook_prepare_commit_msg(message_file: &Path, source: Option<&str>) -> Result<()> {
    clear_pending_sync_if_present()?;

    if env::var("SSMVER_AMENDING").ok().as_deref() == Some("1") {
        return Ok(());
    }

    if matches!(source, Some("merge" | "squash" | "commit")) {
        return Ok(());
    }

    let repo_root = match git_repo_root() {
        Ok(root) => root,
        Err(_) => return Ok(()),
    };
    let config_path = repo_root.join(CONFIG_FILE);
    if !config_path.exists() {
        return Ok(());
    }

    let mut config = SsmverConfig::load(&config_path)?;
    let first_line = fs::read_to_string(message_file)
        .ok()
        .and_then(|content| content.lines().next().map(ToString::to_string))
        .unwrap_or_default();
    let Some(prefix) = extract_commit_prefix(&first_line) else {
        return Ok(());
    };
    let Some(level) = config.prefixes.get(&prefix).copied() else {
        return Ok(());
    };

    let Some(next_version) = compute_commit_bump_version(&repo_root, &config, level)? else {
        return Ok(());
    };
    sync_project_version(&repo_root, &config_path, &mut config, next_version, true)?;

    if should_prompt_for_body(&config, &prefix) && !commit_message_has_body(message_file)? {
        prompt_for_commit_body(message_file, &prefix)?;
    }

    Ok(())
}

fn handle_hook_post_commit() -> Result<()> {
    if env::var("SSMVER_AMENDING").ok().as_deref() == Some("1") {
        return Ok(());
    }

    let repo_root = match git_repo_root() {
        Ok(root) => root,
        Err(_) => return Ok(()),
    };

    let Some(pending) = read_pending_sync(&repo_root)? else {
        return Ok(());
    };
    if pending.files.is_empty() {
        clear_pending_sync(&repo_root)?;
        return Ok(());
    }

    for file in &pending.files {
        run_git(
            &repo_root,
            [
                "add".to_string(),
                "--".to_string(),
                file.to_string_lossy().into_owned(),
            ],
        )?;
    }

    if !has_cached_changes_for_paths(&repo_root, &pending.files)? {
        clear_pending_sync(&repo_root)?;
        return Ok(());
    }

    let status = Command::new("git")
        .args(["commit", "--amend", "--no-edit"])
        .current_dir(&repo_root)
        .env("SSMVER_AMENDING", "1")
        .status()
        .context("failed to amend commit with synchronized version files")?;

    if !status.success() {
        bail!("git commit --amend --no-edit failed");
    }

    clear_pending_sync(&repo_root)?;
    Ok(())
}

fn sync_project_version(
    repo_root: &Path,
    config_path: &Path,
    config: &mut SsmverConfig,
    new_version: Version,
    persist_pending: bool,
) -> Result<Vec<PathBuf>> {
    let targets = discover_targets(repo_root, &config.sync)?;
    ensure_syncable_targets(&targets)?;

    let config_changed = config.version != new_version;
    if config_changed {
        config.version = new_version;
        config.save(config_path)?;
    }

    let mut changed_files = apply_version_to_targets(repo_root, &targets, &config.version)?;
    if config_changed {
        changed_files.push(PathBuf::from(CONFIG_FILE));
    }
    changed_files.sort();
    changed_files.dedup();

    if persist_pending {
        write_pending_sync(repo_root, &changed_files)?;
    }

    Ok(changed_files)
}

fn install_or_refresh_repo(repo_root: &Path) -> Result<Vec<String>> {
    let install_root = repo_root.join(SSMVER_DIR);
    let hooks_dir = install_root.join("hooks");
    let binary_path = env::current_exe()
        .context("failed to resolve current ssmver binary path")?
        .canonicalize()
        .context("failed to canonicalize current ssmver binary path")?;

    let mut summary = Vec::new();
    fs::create_dir_all(&hooks_dir)
        .with_context(|| format!("failed to create {}", hooks_dir.display()))?;
    summary.push(format!("Ensured {}", install_root.display()));

    write_executable(
        &hooks_dir.join("prepare-commit-msg"),
        &hooks::prepare_commit_msg_script(&binary_path),
    )?;
    write_executable(
        &hooks_dir.join("post-commit"),
        &hooks::post_commit_script(&binary_path),
    )?;
    summary.push("Generated hook wrappers".to_string());

    set_hooks_path(repo_root)?;
    summary.push("Set git core.hooksPath to .ssmver/hooks".to_string());

    if ensure_gitignore_has_ssmver(repo_root)? {
        summary.push("Updated .gitignore with .ssmver/".to_string());
    } else {
        summary.push(".gitignore already ignored .ssmver/".to_string());
    }

    Ok(summary)
}

fn compute_commit_bump_version(
    repo_root: &Path,
    config: &SsmverConfig,
    level: BumpLevel,
) -> Result<Option<Version>> {
    match config.settings.mode {
        config::Mode::All => Ok(Some(compute_next_version(&config.version, level))),
        config::Mode::Branch => {
            let Some(main_ref) = find_main_ref(repo_root)? else {
                return Ok(Some(compute_next_version(&config.version, level)));
            };
            let Some(base) = merge_base(repo_root, &main_ref)? else {
                return Ok(Some(compute_next_version(&config.version, level)));
            };
            let subjects = log_subjects_since(repo_root, &format!("{base}..HEAD"))?;
            let highest = highest_prefix_bump(config, &subjects);
            if highest.is_some_and(|existing| bump_priority(level) <= bump_priority(existing)) {
                return Ok(None);
            }

            let base_version = show_file_at_rev(repo_root, &base, Path::new(CONFIG_FILE))?
                .and_then(|content| {
                    SsmverConfig::from_str(&content)
                        .ok()
                        .map(|config| config.version)
                })
                .unwrap_or_else(|| config.version.clone());
            Ok(Some(compute_next_version(&base_version, level)))
        }
    }
}

fn highest_prefix_bump(config: &SsmverConfig, subjects: &[String]) -> Option<BumpLevel> {
    subjects
        .iter()
        .filter_map(|subject| extract_commit_prefix(subject))
        .filter_map(|prefix| config.prefixes.get(&prefix).copied())
        .max_by_key(|level| bump_priority(*level))
}

fn bump_priority(level: BumpLevel) -> u8 {
    match level {
        BumpLevel::Patch => 1,
        BumpLevel::Minor => 2,
        BumpLevel::Major => 3,
    }
}

fn should_prompt_for_body(config: &SsmverConfig, prefix: &str) -> bool {
    match config.settings.prompt {
        PromptMode::Never => false,
        PromptMode::Always => true,
        PromptMode::Ask => config
            .settings
            .prompt_prefixes
            .iter()
            .any(|candidate| candidate == prefix),
    }
}

fn commit_message_has_body(message_file: &Path) -> Result<bool> {
    let content = fs::read_to_string(message_file)?;
    Ok(content.lines().skip(1).any(|line| !line.trim().is_empty()))
}

fn prompt_for_commit_body(message_file: &Path, prefix: &str) -> Result<()> {
    let mut tty_writer = match OpenOptions::new().write(true).open("/dev/tty") {
        Ok(tty) => tty,
        Err(_) => return Ok(()),
    };
    let tty_reader = match OpenOptions::new().read(true).open("/dev/tty") {
        Ok(tty) => tty,
        Err(_) => return Ok(()),
    };

    write!(
        tty_writer,
        "ssmver description for {prefix} commit (optional): "
    )?;
    tty_writer.flush()?;

    let mut line = String::new();
    let mut reader = BufReader::new(tty_reader);
    reader.read_line(&mut line)?;
    let line = line.trim_end();
    if line.is_empty() {
        return Ok(());
    }

    let mut file = OpenOptions::new().append(true).open(message_file)?;
    writeln!(file)?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn ensure_syncable_targets(targets: &[VersionTarget]) -> Result<()> {
    let blockers = blocking_targets(targets);
    if blockers.is_empty() {
        return Ok(());
    }

    let mut lines =
        vec!["Refusing to sync because some supported files are dynamic or invalid:".to_string()];
    for target in blockers {
        lines.push(format!(
            "- {} ({:?}): {}",
            target.path.display(),
            target.target_kind,
            target.detail.as_deref().unwrap_or("unsupported target")
        ));
    }
    bail!(lines.join("\n"))
}

fn pending_sync_path(repo_root: &Path) -> PathBuf {
    repo_root.join(SSMVER_DIR).join(hooks::PENDING_SYNC_FILE)
}

fn write_pending_sync(repo_root: &Path, files: &[PathBuf]) -> Result<()> {
    fs::create_dir_all(repo_root.join(SSMVER_DIR))?;
    let pending = PendingSync {
        files: files.to_vec(),
    };
    fs::write(
        pending_sync_path(repo_root),
        serde_json::to_vec_pretty(&pending)?,
    )?;
    Ok(())
}

fn read_pending_sync(repo_root: &Path) -> Result<Option<PendingSync>> {
    let path = pending_sync_path(repo_root);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read(&path)?;
    Ok(Some(serde_json::from_slice(&content)?))
}

fn clear_pending_sync(repo_root: &Path) -> Result<()> {
    let path = pending_sync_path(repo_root);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn clear_pending_sync_if_present() -> Result<()> {
    if let Ok(repo_root) = git_repo_root() {
        clear_pending_sync(&repo_root)?;
    }
    Ok(())
}

fn has_cached_changes_for_paths(repo_root: &Path, paths: &[PathBuf]) -> Result<bool> {
    let mut command = Command::new("git");
    command.args(["diff", "--cached", "--quiet", "HEAD", "--"]);
    for path in paths {
        command.arg(path);
    }
    let status = command.current_dir(repo_root).status()?;
    Ok(!status.success())
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
