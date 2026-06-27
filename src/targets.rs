use std::{
    borrow::Cow,
    ffi::OsStr,
    fmt,
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use ignore::WalkBuilder;
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use semver::Version;
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use toml_edit::{value, DocumentMut, Item};
use xmltree::{Element, XMLNode};

use crate::config::SyncSettings;

static COMMIT_PREFIX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([A-Za-z0-9_-]+)(?:\([^)]*\))?:").unwrap());
static VERSION_ASSIGNMENT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^version\s*=").unwrap());
static GRADLE_LITERAL_VERSION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^version\s*=\s*(?:"(?P<value_dq>[^"\n]+)"|'(?P<value_sq>[^'\n]+)')\s*(?://.*)?$"#).unwrap()
});
static SETUP_PY_LITERAL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"version\s*=\s*(?:"(?P<value_dq>[^"]+)"|'(?P<value_sq>[^']+)')"#).unwrap()
});
static PYTHON_DUNDER_VERSION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^__version__\s*=\s*(?:"(?P<value_dq>[^"\n]+)"|'(?P<value_sq>[^'\n]+)')\s*$"#).unwrap()
});
static RUBY_GEMSPEC_VERSION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^\s*spec\.version\s*=\s*(?:"(?P<value_dq>[^"\n]+)"|'(?P<value_sq>[^'\n]+)')\s*$"#,).unwrap()
});
static RUBY_VERSION_RB_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^\s*VERSION\s*=\s*(?:"(?P<value_dq>[^"\n]+)"|'(?P<value_sq>[^'\n]+)')\s*$"#).unwrap()
});
static DOTNET_ASSEMBLY_VERSION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"AssemblyVersion\((?:"(?P<value_dq>[^"]+)"|'(?P<value_sq>[^']+)')\)"#).unwrap()
});
static DOTNET_FILE_VERSION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"AssemblyFileVersion\((?:"(?P<value_dq>[^"]+)"|'(?P<value_sq>[^']+)')\)"#).unwrap()
});
static DOTNET_INFO_VERSION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"AssemblyInformationalVersion\((?:"(?P<value_dq>[^"]+)"|'(?P<value_sq>[^']+)')\)"#).unwrap()
});
static XCODE_MARKETING_VERSION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^\s*MARKETING_VERSION\s*=\s*(?P<value_dq>[0-9]+\.[0-9]+(?:\.[0-9]+)?)\s*;"#).unwrap()
});
static PLIST_BUNDLE_VERSION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)<key>CFBundleShortVersionString</key>\s*<string>(?P<value_dq>[^<]+)</string>"#).unwrap()
});

const SKIP_DIRS: &[&str] = &[".git", ".ssmver", ".gradle", "node_modules", "target", "dist", "build", "vendor"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    Cargo,
    Node,
    Tauri,
    Maven,
    Gradle,
    Python,
    Dotnet,
    Ruby,
    Php,
    Swift,
}

impl fmt::Display for Ecosystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cargo  => f.write_str("Cargo"),
            Self::Node   => f.write_str("Node"),
            Self::Tauri  => f.write_str("Tauri"),
            Self::Maven  => f.write_str("Maven"),
            Self::Gradle => f.write_str("Gradle"),
            Self::Python => f.write_str("Python"),
            Self::Dotnet => f.write_str(".NET"),
            Self::Ruby   => f.write_str("Ruby"),
            Self::Php    => f.write_str("PHP"),
            Self::Swift  => f.write_str("Swift"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    CargoToml,
    CargoLock,
    PackageJson,
    NpmLockfile,
    TauriConfigJson,
    MavenPom,
    GradleProperties,
    GradleBuildScript,
    PyprojectToml,
    SetupCfg,
    SetupPy,
    PythonVersionConstant,
    DotnetProject,
    DotnetAssemblyInfo,
    RubyGemspec,
    RubyVersionFile,
    ComposerJson,
    XcodeProject,
    InfoPlist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetAuthority {
    Literal,
    WorkspaceRoot,
    Mirror,
    CuratedConstant,
    LocalParent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetStatus {
    Managed,
    SkippedDynamic,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionTarget {
    pub ecosystem: Ecosystem,
    pub path: PathBuf,
    pub target_kind: TargetKind,
    pub current_version: Option<String>,
    pub authority: TargetAuthority,
    pub status: TargetStatus,
    pub detail: Option<String>,
}

impl VersionTarget {
    fn managed(ecosystem: Ecosystem, path: &Path, target_kind: TargetKind, authority: TargetAuthority, version: Version) -> Self {
        Self {
            ecosystem,
            path: path.to_path_buf(),
            target_kind,
            current_version: Some(version.to_string()),
            authority,
            status: TargetStatus::Managed,
            detail: None,
        }
    }

    fn issue(ecosystem: Ecosystem, path: &Path, target_kind: TargetKind, authority: TargetAuthority, status: TargetStatus, current_version: Option<String>, detail: impl Into<String>) -> Self {
        Self {
            ecosystem,
            path: path.to_path_buf(),
            target_kind,
            current_version,
            authority,
            status,
            detail: Some(detail.into()),
        }
    }

    pub fn is_blocking(&self) -> bool {
        matches!(self.status, TargetStatus::SkippedDynamic | TargetStatus::Error)
    }
}

pub fn discover_targets(repo_root: &Path, sync: &SyncSettings) -> Result<Vec<VersionTarget>> {
    let walker = WalkBuilder::new(repo_root).hidden(false).git_ignore(true).git_global(true).git_exclude(true).build();
    let mut targets = Vec::new();
    for entry in walker {
        let entry = entry?;
        if !entry.file_type().map(|file_type| file_type.is_file()).unwrap_or(false) {
            continue;
        }

        let path = entry.path();
        let relative = path.strip_prefix(repo_root).with_context(|| format!("failed to make {} relative to repo root", path.display()))?;
        if should_skip_path(relative) {
            continue;
        }

        let excluded = is_excluded(relative, &sync.exclude);
        targets.extend(detect_targets_for_file(repo_root, relative, excluded)?);
    }

    targets.sort_by(|left, right| {left.path.cmp(&right.path).then_with(|| {format!("{:?}", left.target_kind).cmp(&format!("{:?}", right.target_kind))})});
    Ok(targets)
}

pub fn apply_version_to_targets(repo_root: &Path, targets: &[VersionTarget], new_version: &Version) -> Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    for target in targets {
        if target.status != TargetStatus::Managed {
            continue;
        }

        let did_change = match target.target_kind {
            TargetKind::CargoToml => update_cargo_toml(&repo_root.join(&target.path), new_version)?,
            TargetKind::CargoLock => update_cargo_lock(&repo_root.join(&target.path), new_version)?,
            TargetKind::PackageJson => {
                update_package_json(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::NpmLockfile => {
                update_npm_lockfile(repo_root, &target.path, targets, new_version)?
            }
            TargetKind::TauriConfigJson => {
                update_tauri_config_json(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::MavenPom => update_maven_pom(repo_root, &target.path, new_version)?,
            TargetKind::GradleProperties => {
                update_gradle_properties(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::GradleBuildScript => {
                update_gradle_build_script(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::PyprojectToml => {
                update_pyproject_toml(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::SetupCfg => update_setup_cfg(&repo_root.join(&target.path), new_version)?,
            TargetKind::SetupPy => update_setup_py(&repo_root.join(&target.path), new_version)?,
            TargetKind::PythonVersionConstant => {
                update_python_dunder_version(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::DotnetProject => {
                update_dotnet_project(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::DotnetAssemblyInfo => {
                update_dotnet_assembly_info(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::RubyGemspec => {
                update_ruby_gemspec(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::RubyVersionFile => {
                update_ruby_version_file(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::ComposerJson => {
                update_composer_json(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::XcodeProject => {
                update_xcode_project(&repo_root.join(&target.path), new_version)?
            }
            TargetKind::InfoPlist => {
                update_info_plist(&repo_root.join(&target.path), new_version)?
            }
        };

        if did_change {
            changed.push(target.path.clone());
        }
    }

    changed.sort();
    changed.dedup();
    Ok(changed)
}

pub fn blocking_targets<'a>(targets: &'a [VersionTarget]) -> Vec<&'a VersionTarget> {
    targets.iter().filter(|target| target.is_blocking()).collect()
}

pub fn infer_seed_version(targets: &[VersionTarget]) -> Result<Option<Version>> {
    let versions = targets
        .iter()
        .filter(|target| target.status == TargetStatus::Managed)
        .filter_map(|target| target.current_version.as_deref())
        .map(Version::parse)
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut unique = versions.into_iter().map(|version| version.to_string()).collect::<Vec<_>>();
    unique.sort();
    unique.dedup();

    match unique.len() {
        0 => Ok(None),
        1 => Ok(Some(Version::parse(&unique[0])?)),
        _ => bail!("Detected conflicting versions: {}", unique.join(", ")),
    }
}

pub fn format_target_table(targets: &[VersionTarget]) -> String {
    let mut lines = Vec::new();
    for target in targets {
        let version = target.current_version.as_deref().unwrap_or("-");
        let detail = target.detail.as_deref().unwrap_or("");
        lines.push(format!("{:<16} {:<22} {:<16} {:<14} {}", path_display(&target.path), format!("{:?}", target.target_kind), version, format!("{:?}", target.status), detail));
    }
    lines.join("\n")
}

pub fn extract_commit_prefix(subject: &str) -> Option<String> {
    COMMIT_PREFIX_RE.captures(subject.trim()).and_then(|captures| captures.get(1).map(|value| value.as_str().to_string()))
}

fn detect_targets_for_file(repo_root: &Path, relative_path: &Path, excluded: bool) -> Result<Vec<VersionTarget>> {
    if excluded {
        return Ok(Vec::new());
    }

    let file_name = relative_path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    let targets = match file_name {
        "Cargo.toml"                                    => detect_cargo_toml(repo_root, relative_path)?,
        "Cargo.lock"                                    => detect_cargo_lock(repo_root, relative_path)?,
        "package.json"                                  => detect_package_json(repo_root, relative_path)?,
        "package-lock.json" | "npm-shrinkwrap.json"     => detect_npm_lockfile(repo_root, relative_path)?,
        "tauri.conf.json"                               => detect_tauri_config_json(repo_root, relative_path)?,
        "pom.xml"                                       => detect_maven_pom(repo_root, relative_path)?,
        "gradle.properties"                             => detect_gradle_properties(repo_root, relative_path)?,
        "build.gradle" | "build.gradle.kts"             => detect_gradle_build_script(repo_root, relative_path)?,
        "pyproject.toml"                                => detect_pyproject_toml(repo_root, relative_path)?,
        "setup.cfg"                                     => detect_setup_cfg(repo_root, relative_path)?,
        "setup.py"                                      => detect_setup_py(repo_root, relative_path)?,
        "__init__.py" | "version.py"                    => detect_python_dunder_version(repo_root, relative_path)?,
        "Directory.Build.props"                         => detect_dotnet_project(repo_root, relative_path)?,
        "AssemblyInfo.cs"                               => detect_dotnet_assembly_info(repo_root, relative_path)?,
        "composer.json"                                 => detect_composer_json(repo_root, relative_path)?,
        other if other.ends_with(".csproj")       => detect_dotnet_project(repo_root, relative_path)?,
        other if other.ends_with(".gemspec")      => detect_ruby_gemspec(repo_root, relative_path)?,
        "project.pbxproj"                               => detect_xcode_project(repo_root, relative_path)?,
        "Info.plist"                                    => detect_info_plist(repo_root, relative_path)?,
        "version.rb"                                    => detect_ruby_version_file(repo_root, relative_path)?,
        _                                               => Vec::new(),
    };

    Ok(targets)
}

fn detect_cargo_toml(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let doc = parse_toml_document(&content, relative_path)?;

    let package_version = item_string(get_item(&doc, &["package", "version"]));
    let workspace_version = item_string(get_item(&doc, &["workspace", "package", "version"]));
    let inherited = item_bool(get_item(&doc, &["package", "version", "workspace"])).unwrap_or(false);
    let has_package_version = package_version.is_some();
    let has_workspace_version = workspace_version.is_some();

    if package_version.is_none() && workspace_version.is_none() && !inherited {
        return Ok(Vec::new());
    }

    let mut versions = Vec::new();
    if let Some(package_version) = package_version {
        versions.push(package_version);
    }
    if let Some(workspace_version) = workspace_version {
        versions.push(workspace_version);
    }

    if versions.is_empty() {
        return Ok(Vec::new());
    }

    let authority = if !has_package_version && has_workspace_version {
        TargetAuthority::WorkspaceRoot
    } else {
        TargetAuthority::Literal
    };

    let parsed = parse_consistent_versions(Ecosystem::Cargo, relative_path, TargetKind::CargoToml, authority, &versions)?;

    Ok(vec![parsed])
}

fn detect_cargo_lock(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let doc = parse_toml_document(&content, relative_path)?;
    let versions = cargo_lock_local_package_versions(&doc);

    if versions.is_empty() {
        return Ok(Vec::new());
    }

    Ok(vec![parse_consistent_versions(Ecosystem::Cargo, relative_path, TargetKind::CargoLock, TargetAuthority::Mirror, &versions)?])
}

fn cargo_lock_local_package_versions(doc: &DocumentMut) -> Vec<String> {
    let Some(packages) = doc["package"].as_array_of_tables() else {
        return Vec::new();
    };

    packages
        .iter()
        .filter(|package| package.get("source").is_none())
        .filter_map(|package| item_string(package.get("version")))
        .collect()
}

fn detect_package_json(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let json: JsonValue = serde_json::from_str(&content).with_context(|| format!("failed to parse {}", relative_path.display()))?;
    let Some(version) = json.get("version").and_then(JsonValue::as_str) else {
        return Ok(Vec::new());
    };

    Ok(vec![VersionTarget::managed(Ecosystem::Node, relative_path, TargetKind::PackageJson, TargetAuthority::Literal, parse_version(Ecosystem::Node, relative_path, TargetKind::PackageJson, TargetAuthority::Literal, version)?)])
}

fn detect_npm_lockfile(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let json: JsonValue = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", relative_path.display()))?;

    let root_version = json.get("version").and_then(JsonValue::as_str);
    let packages_root_version = json
        .get("packages")
        .and_then(JsonValue::as_object)
        .and_then(|packages| packages.get(""))
        .and_then(JsonValue::as_object)
        .and_then(|root| root.get("version"))
        .and_then(JsonValue::as_str);

    if root_version.is_none() && packages_root_version.is_none() {
        return Ok(Vec::new());
    }

    let versions = [root_version, packages_root_version]
        .into_iter()
        .flatten()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    Ok(vec![parse_consistent_versions(Ecosystem::Node, relative_path, TargetKind::NpmLockfile, TargetAuthority::Mirror, &versions)?])
}

fn detect_tauri_config_json(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let json: JsonValue = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", relative_path.display()))?;
    let Some(version) = json.get("version").and_then(JsonValue::as_str) else {
        return Ok(Vec::new());
    };

    Ok(vec![VersionTarget::managed(Ecosystem::Tauri, relative_path, TargetKind::TauriConfigJson, TargetAuthority::Literal, parse_version(Ecosystem::Tauri, relative_path, TargetKind::TauriConfigJson, TargetAuthority::Literal, version)?)])
}

fn detect_maven_pom(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let root = parse_xml(&content, relative_path)?;

    let project_version = child_element(&root, "version")
        .and_then(Element::get_text)
        .map(|value| value.into_owned());
    let parent = child_element(&root, "parent");
    let local_parent_version = parent
        .and_then(|parent| {
            if resolves_local_maven_parent(repo_root, relative_path, parent) {
                child_element(parent, "version")
            } else {
                None
            }
        })
        .and_then(Element::get_text)
        .map(|value| value.into_owned());

    if project_version.is_none() && local_parent_version.is_none() {
        return Ok(Vec::new());
    }

    let versions = [project_version, local_parent_version]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Ok(vec![parse_consistent_versions_with_dynamic(Ecosystem::Maven, relative_path, TargetKind::MavenPom, TargetAuthority::LocalParent, &versions)?])
}

fn detect_gradle_properties(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let version_lines = content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('#') || trimmed.starts_with('!') || !trimmed.starts_with("version") {
                return None;
            }
            trimmed.split_once('=').map(|(_, value)| value.trim().to_string())}).collect::<Vec<_>>();

    if version_lines.is_empty() {
        return Ok(Vec::new());
    }
    if version_lines.len() > 1 {
        return Ok(vec![VersionTarget::issue(Ecosystem::Gradle, relative_path, TargetKind::GradleProperties, TargetAuthority::Literal, TargetStatus::Error, None, "multiple version= assignments found")]);
    }

    let version = &version_lines[0];
    if version.contains("${") {
        return Ok(vec![VersionTarget::issue(Ecosystem::Gradle, relative_path, TargetKind::GradleProperties, TargetAuthority::Literal, TargetStatus::SkippedDynamic, Some(version.clone()), "dynamic gradle.properties version is not supported")]);
    }

    Ok(vec![VersionTarget::managed(Ecosystem::Gradle, relative_path, TargetKind::GradleProperties, TargetAuthority::Literal, parse_version(Ecosystem::Gradle, relative_path, TargetKind::GradleProperties, TargetAuthority::Literal, version)?)])
}

fn detect_gradle_build_script(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let sibling_properties = relative_path.parent().unwrap_or_else(|| Path::new("")).join("gradle.properties");
    if repo_root.join(&sibling_properties).is_file() {
        let content = fs::read_to_string(repo_root.join(&sibling_properties))?;
        if content.lines().any(|line| line.trim_start().starts_with("version") && line.contains('=')) {
            return Ok(Vec::new());
        }
    }

    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let literal_matches = GRADLE_LITERAL_VERSION_RE
        .captures_iter(&content)
        .map(|captures| capture_regex_value(&captures).unwrap().to_string())
        .collect::<Vec<_>>();
    let generic_matches = VERSION_ASSIGNMENT_RE.find_iter(&content).count();

    if literal_matches.is_empty() && generic_matches == 0 {
        return Ok(Vec::new());
    }
    if generic_matches > literal_matches.len() {
        return Ok(vec![VersionTarget::issue(Ecosystem::Gradle, relative_path, TargetKind::GradleBuildScript, TargetAuthority::Literal, TargetStatus::SkippedDynamic, None, "dynamic or unsupported version assignment found in Gradle build script")]);
    }
    if literal_matches.len() != 1 {
        return Ok(vec![VersionTarget::issue(Ecosystem::Gradle, relative_path, TargetKind::GradleBuildScript, TargetAuthority::Literal, TargetStatus::Error, None, "expected exactly one top-level literal version assignment")]);
    }

    Ok(vec![VersionTarget::managed(Ecosystem::Gradle, relative_path, TargetKind::GradleBuildScript, TargetAuthority::Literal, parse_version(Ecosystem::Gradle, relative_path, TargetKind::GradleBuildScript, TargetAuthority::Literal, &literal_matches[0])?)])
}

fn detect_pyproject_toml(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let doc = parse_toml_document(&content, relative_path)?;

    if dynamic_array_contains_version(get_item(&doc, &["project", "dynamic"])) {
        return Ok(vec![VersionTarget::issue(Ecosystem::Python, relative_path, TargetKind::PyprojectToml, TargetAuthority::Literal, TargetStatus::SkippedDynamic, None, "pyproject.toml declares dynamic project.version")]);
    }

    let project_version = item_string(get_item(&doc, &["project", "version"]));
    let poetry_version = item_string(get_item(&doc, &["tool", "poetry", "version"]));

    if project_version.is_none() && poetry_version.is_none() {
        return Ok(Vec::new());
    }

    let versions = [project_version, poetry_version].into_iter().flatten().collect::<Vec<_>>();
    Ok(vec![parse_consistent_versions(Ecosystem::Python, relative_path, TargetKind::PyprojectToml, TargetAuthority::Literal, &versions)?])
}

fn detect_setup_cfg(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let Some(value) = ini_value(&content, "metadata", "version") else {
        return Ok(Vec::new());
    };

    if value.contains('%') || value.contains('{') {
        return Ok(vec![VersionTarget::issue(Ecosystem::Python, relative_path, TargetKind::SetupCfg, TargetAuthority::Literal, TargetStatus::SkippedDynamic, Some(value), "dynamic setup.cfg metadata.version is not supported")]);
    }

    Ok(vec![VersionTarget::managed(Ecosystem::Python, relative_path, TargetKind::SetupCfg, TargetAuthority::Literal, parse_version(Ecosystem::Python, relative_path, TargetKind::SetupCfg, TargetAuthority::Literal, &value)?)])
}

fn detect_setup_py(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let literal_matches = SETUP_PY_LITERAL_RE.captures_iter(&content).map(|captures| capture_regex_value(&captures).unwrap().to_string()).collect::<Vec<_>>();
    let generic_matches = Regex::new(r"version\s*=").unwrap().find_iter(&content).count();

    if literal_matches.is_empty() && generic_matches == 0 {
        return Ok(Vec::new());
    }
    if generic_matches > literal_matches.len() {
        return Ok(vec![VersionTarget::issue(Ecosystem::Python, relative_path, TargetKind::SetupPy, TargetAuthority::Literal, TargetStatus::SkippedDynamic, None, "dynamic setup.py version is not supported")]);
    }
    if literal_matches.len() != 1 {
        return Ok(vec![VersionTarget::issue(Ecosystem::Python, relative_path, TargetKind::SetupPy, TargetAuthority::Literal, TargetStatus::Error, None, "expected exactly one literal setup.py version assignment")]);
    }

    Ok(vec![VersionTarget::managed(Ecosystem::Python, relative_path, TargetKind::SetupPy, TargetAuthority::Literal, parse_version(Ecosystem::Python, relative_path, TargetKind::SetupPy, TargetAuthority::Literal, &literal_matches[0])?)])
}

fn detect_python_dunder_version(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let literal_matches = PYTHON_DUNDER_VERSION_RE.captures_iter(&content).map(|captures| capture_regex_value(&captures).unwrap().to_string()).collect::<Vec<_>>();
    let generic_matches = Regex::new(r"(?m)^__version__\s*=").unwrap().find_iter(&content).count();

    if literal_matches.is_empty() && generic_matches == 0 {
        return Ok(Vec::new());
    }
    if generic_matches > literal_matches.len() {
        return Ok(vec![VersionTarget::issue(Ecosystem::Python, relative_path, TargetKind::PythonVersionConstant, TargetAuthority::CuratedConstant, TargetStatus::SkippedDynamic, None, "dynamic __version__ assignment is not supported")]);
    }
    if literal_matches.len() != 1 {
        return Ok(vec![VersionTarget::issue(Ecosystem::Python, relative_path, TargetKind::PythonVersionConstant, TargetAuthority::CuratedConstant, TargetStatus::Error, None, "expected exactly one literal __version__ assignment")]);
    }

    Ok(vec![VersionTarget::managed(Ecosystem::Python, relative_path, TargetKind::PythonVersionConstant, TargetAuthority::CuratedConstant, parse_version(Ecosystem::Python, relative_path, TargetKind::PythonVersionConstant, TargetAuthority::CuratedConstant, &literal_matches[0])?)])
}

fn detect_dotnet_project(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let root = parse_xml(&content, relative_path)?;
    let mut values = Vec::new();
    collect_texts_by_name(&root, "Version", &mut values);

    if values.is_empty() {
        return Ok(Vec::new());
    }
    if values.iter().any(|value| value.contains("$(") || value.contains("@(")) {
        return Ok(vec![VersionTarget::issue(Ecosystem::Dotnet, relative_path, TargetKind::DotnetProject, TargetAuthority::Literal, TargetStatus::SkippedDynamic, None, "dynamic <Version> value is not supported")]);
    }

    Ok(vec![parse_consistent_versions(Ecosystem::Dotnet, relative_path, TargetKind::DotnetProject, TargetAuthority::Literal, &values)?])
}

fn detect_dotnet_assembly_info(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let assembly = DOTNET_ASSEMBLY_VERSION_RE.captures(&content).map(|captures| capture_regex_value(&captures).unwrap().to_string());
    let file = DOTNET_FILE_VERSION_RE.captures(&content).map(|captures| capture_regex_value(&captures).unwrap().to_string());
    let informational = DOTNET_INFO_VERSION_RE.captures(&content).map(|captures| capture_regex_value(&captures).unwrap().to_string());

    if assembly.is_none() && file.is_none() && informational.is_none() {
        return Ok(Vec::new());
    }

    let mut normalized = Vec::new();
    if let Some(value) = assembly.as_deref() {
        normalized.push(normalize_dotnet_numeric_version(value).ok_or_else(|| {anyhow!("unsupported AssemblyVersion format in {}", relative_path.display())})?);
    }
    if let Some(value) = file.as_deref() {
        normalized.push(normalize_dotnet_numeric_version(value).ok_or_else(|| {anyhow!("unsupported AssemblyFileVersion format in {}", relative_path.display())})?);
    }
    if let Some(value) = informational.as_deref() {
        normalized.push(value.to_string());
    }

    Ok(vec![parse_consistent_versions(Ecosystem::Dotnet, relative_path, TargetKind::DotnetAssemblyInfo, TargetAuthority::CuratedConstant, &normalized)?])
}

fn detect_ruby_gemspec(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let literal_matches = RUBY_GEMSPEC_VERSION_RE.captures_iter(&content).map(|captures| capture_regex_value(&captures).unwrap().to_string()).collect::<Vec<_>>();
    let generic_matches = Regex::new(r"(?m)^\s*spec\.version\s*=").unwrap().find_iter(&content).count();

    if literal_matches.is_empty() && generic_matches == 0 {
        return Ok(Vec::new());
    }
    if generic_matches > literal_matches.len() {
        return Ok(vec![VersionTarget::issue(Ecosystem::Ruby, relative_path, TargetKind::RubyGemspec, TargetAuthority::Literal, TargetStatus::SkippedDynamic, None, "dynamic gemspec version is not supported")]);
    }
    if literal_matches.len() != 1 {
        return Ok(vec![VersionTarget::issue(Ecosystem::Ruby, relative_path, TargetKind::RubyGemspec, TargetAuthority::Literal, TargetStatus::Error, None, "expected exactly one literal gemspec version assignment")]);
    }

    Ok(vec![VersionTarget::managed(Ecosystem::Ruby, relative_path, TargetKind::RubyGemspec, TargetAuthority::Literal, parse_version(Ecosystem::Ruby, relative_path, TargetKind::RubyGemspec, TargetAuthority::Literal, &literal_matches[0])?)])
}

fn detect_ruby_version_file(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    if !path_contains_component(relative_path, "lib") {
        return Ok(Vec::new());
    }

    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let literal_matches = RUBY_VERSION_RB_RE.captures_iter(&content).map(|captures| capture_regex_value(&captures).unwrap().to_string()).collect::<Vec<_>>();
    let generic_matches = Regex::new(r"(?m)^\s*VERSION\s*=").unwrap().find_iter(&content).count();

    if literal_matches.is_empty() && generic_matches == 0 {
        return Ok(Vec::new());
    }
    if generic_matches > literal_matches.len() {
        return Ok(vec![VersionTarget::issue(Ecosystem::Ruby, relative_path, TargetKind::RubyVersionFile, TargetAuthority::CuratedConstant, TargetStatus::SkippedDynamic, None, "dynamic Ruby VERSION constant is not supported")]);
    }
    if literal_matches.len() != 1 {
        return Ok(vec![VersionTarget::issue(Ecosystem::Ruby, relative_path, TargetKind::RubyVersionFile, TargetAuthority::CuratedConstant, TargetStatus::Error, None, "expected exactly one literal Ruby VERSION constant")]);
    }

    Ok(vec![VersionTarget::managed(Ecosystem::Ruby, relative_path, TargetKind::RubyVersionFile, TargetAuthority::CuratedConstant, parse_version(Ecosystem::Ruby, relative_path, TargetKind::RubyVersionFile, TargetAuthority::CuratedConstant, &literal_matches[0])?)])
}

fn detect_xcode_project(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    if !path_contains_component(relative_path, "project.pbxproj") && !relative_path.components().any(|c| matches!(c, Component::Normal(name) if name.to_string_lossy().ends_with(".xcodeproj"))) {
        return Ok(Vec::new());
    }

    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let literal_matches: Vec<String> = XCODE_MARKETING_VERSION_RE
        .captures_iter(&content)
        .filter_map(|captures| captures.name("value_dq").map(|m| m.as_str().to_string()))
        .collect();
    let generic_count = Regex::new(r"(?m)^\s*MARKETING_VERSION\s*=")
        .unwrap()
        .find_iter(&content)
        .count();

    if literal_matches.is_empty() && generic_count == 0 {
        return Ok(Vec::new());
    }
    if generic_count > literal_matches.len() {
        return Ok(vec![VersionTarget::issue(Ecosystem::Swift, relative_path, TargetKind::XcodeProject, TargetAuthority::Literal, TargetStatus::SkippedDynamic, None, "dynamic or unsupported MARKETING_VERSION assignment")]);
    }

    let mut unique: Vec<String> = literal_matches.clone();
    unique.sort();
    unique.dedup();
    if unique.len() > 1 {
        return Ok(vec![VersionTarget::issue(Ecosystem::Swift, relative_path, TargetKind::XcodeProject, TargetAuthority::Literal, TargetStatus::Error, None, format!("conflicting MARKETING_VERSION values: {}", unique.join(", ")))]);
    }

    let raw = &unique[0];
    let normalized = if raw.matches('.').count() == 1 {
        format!("{raw}.0")
    } else {
        raw.to_string()
    };

    Ok(vec![VersionTarget::managed(Ecosystem::Swift, relative_path, TargetKind::XcodeProject, TargetAuthority::Literal, parse_version(Ecosystem::Swift, relative_path, TargetKind::XcodeProject, TargetAuthority::Literal, &normalized)?)])
}

fn detect_info_plist(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;

    let Some(captures) = PLIST_BUNDLE_VERSION_RE.captures(&content) else {
        return Ok(Vec::new());
    };
    let value = captures.name("value_dq").unwrap().as_str();

    if looks_dynamic(value) {
        return Ok(Vec::new());
    }

    let normalized = if value.matches('.').count() == 1 {
        format!("{value}.0")
    } else {
        value.to_string()
    };

    Ok(vec![VersionTarget::managed(Ecosystem::Swift, relative_path, TargetKind::InfoPlist, TargetAuthority::Literal, parse_version(Ecosystem::Swift, relative_path, TargetKind::InfoPlist, TargetAuthority::Literal, &normalized)?)])
}

fn detect_composer_json(repo_root: &Path, relative_path: &Path) -> Result<Vec<VersionTarget>> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let json: JsonValue = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", relative_path.display()))?;
    let Some(version) = json.get("version").and_then(JsonValue::as_str) else {
        return Ok(Vec::new());
    };

    Ok(vec![VersionTarget::managed(Ecosystem::Php, relative_path, TargetKind::ComposerJson, TargetAuthority::Literal, parse_version(Ecosystem::Php, relative_path, TargetKind::ComposerJson, TargetAuthority::Literal, version)?)])
}

fn update_cargo_toml(path: &Path, new_version: &Version) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let mut doc = parse_toml_document(&content, path)?;
    let version_string = new_version.to_string();
    let mut changed = false;

    if item_string(get_item(&doc, &["package", "version"])).is_some() {
        doc["package"]["version"] = value(version_string.clone());
        changed = true;
    }
    if item_string(get_item(&doc, &["workspace", "package", "version"])).is_some() {
        doc["workspace"]["package"]["version"] = value(version_string);
        changed = true;
    }

    if !changed {
        return Ok(false);
    }

    let updated = doc.to_string();
    if updated == content {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn update_cargo_lock(path: &Path, new_version: &Version) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let mut doc = parse_toml_document(&content, path)?;
    let version_string = new_version.to_string();
    let mut changed = false;

    if let Some(packages) = doc["package"].as_array_of_tables_mut() {
        for package in packages.iter_mut() {
            if package.get("source").is_some() {
                continue;
            }
            if item_string(package.get("version")).is_some() {
                package["version"] = value(version_string.clone());
                changed = true;
            }
        }
    }

    if !changed {
        return Ok(false);
    }

    let updated = doc.to_string();
    if updated == content {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn update_package_json(path: &Path, new_version: &Version) -> Result<bool> {
    update_json_version_field(path, new_version, |json| json.get("version").is_some())
}

fn update_npm_lockfile(repo_root: &Path, relative_path: &Path, all_targets: &[VersionTarget], new_version: &Version) -> Result<bool> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let mut json: JsonValue = serde_json::from_str(&content).with_context(|| format!("failed to parse {}", relative_path.display()))?;
    let Some(root) = json.as_object_mut() else {
        bail!("{} is not a JSON object", relative_path.display());
    };
    let version_string = new_version.to_string();
    let mut changed = false;

    if root.contains_key("version") {
        root.insert("version".to_string(), JsonValue::String(version_string.clone()));
        changed = true;
    }

    if let Some(packages) = root.get_mut("packages").and_then(JsonValue::as_object_mut) {
        if let Some(root_package) = packages.get_mut("").and_then(JsonValue::as_object_mut) {
            root_package.insert("version".to_string(), JsonValue::String(version_string.clone()));
            changed = true;
        }

        let lock_dir = relative_path.parent().unwrap_or_else(|| Path::new(""));
        for target in all_targets.iter().filter(|target| target.target_kind == TargetKind::PackageJson) {
            let package_path = target.path.parent().unwrap_or_else(|| Path::new(""));
            if let Ok(stripped) = package_path.strip_prefix(lock_dir) {
                let key = path_display(stripped);
                if key.is_empty() {
                    continue;
                }
                if let Some(package_entry) = packages.get_mut(&key).and_then(JsonValue::as_object_mut) {
                    package_entry.insert("version".to_string(), JsonValue::String(version_string.clone()));
                    changed = true;
                }
            }
        }
    }

    if !changed {
        return Ok(false);
    }
    let updated = serde_json::to_string_pretty(&json)?;
    if updated == content {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn update_tauri_config_json(path: &Path, new_version: &Version) -> Result<bool> {
    update_json_version_field(path, new_version, |json| json.get("version").is_some())
}

fn update_maven_pom(repo_root: &Path, relative_path: &Path, new_version: &Version) -> Result<bool> {
    let path = repo_root.join(relative_path);
    let content = fs::read_to_string(&path)?;
    let mut root = parse_xml(&content, relative_path)?;
    let version_string = new_version.to_string();
    let mut changed = false;

    if let Some(project_version) = child_element_mut(&mut root, "version") {
        set_element_text(project_version, &version_string);
        changed = true;
    }

    if let Some(parent) = child_element_mut(&mut root, "parent") {
        if resolves_local_maven_parent(repo_root, relative_path, parent) {
            if let Some(parent_version) = child_element_mut(parent, "version") {
                set_element_text(parent_version, &version_string);
                changed = true;
            }
        }
    }

    if !changed {
        return Ok(false);
    }

    write_xml_if_changed(&path, &root, &content)
}

fn update_gradle_properties(path: &Path, new_version: &Version) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let version_string = new_version.to_string();
    let mut replaced = false;
    let updated = content.lines().map(|line| {
            let trimmed = line.trim_start();
            if !replaced && trimmed.starts_with("version") && line.contains('=') {
                replaced = true;
                let prefix = line.split_once('=').map(|(prefix, _)| prefix).unwrap_or("version");
                format!("{prefix}={version_string}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let updated = if content.ends_with('\n') {
        format!("{updated}\n")
    } else {
        updated
    };

    if !replaced || updated == content {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn update_gradle_build_script(path: &Path, new_version: &Version) -> Result<bool> {
    replace_single_capture(path, &GRADLE_LITERAL_VERSION_RE, &new_version.to_string())
}

fn update_pyproject_toml(path: &Path, new_version: &Version) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let mut doc = parse_toml_document(&content, path)?;
    let version_string = new_version.to_string();
    let mut changed = false;
    if item_string(get_item(&doc, &["project", "version"])).is_some() {
        doc["project"]["version"] = value(version_string.clone());
        changed = true;
    }
    if item_string(get_item(&doc, &["tool", "poetry", "version"])).is_some() {
        doc["tool"]["poetry"]["version"] = value(version_string);
        changed = true;
    }

    if !changed {
        return Ok(false);
    }
    let updated = doc.to_string();
    if updated == content {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn update_setup_cfg(path: &Path, new_version: &Version) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let Some((start, end, prefix)) = ini_value_span(&content, "metadata", "version") else {
        return Ok(false);
    };
    let updated = format!("{}{}{}\n{}", &content[..start], prefix, new_version, &content[end..]);
    let updated = normalize_extra_newline(updated);
    if updated == content {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn update_setup_py(path: &Path, new_version: &Version) -> Result<bool> {
    replace_single_capture(path, &SETUP_PY_LITERAL_RE, &new_version.to_string())
}

fn update_python_dunder_version(path: &Path, new_version: &Version) -> Result<bool> {
    replace_single_capture(path, &PYTHON_DUNDER_VERSION_RE, &new_version.to_string())
}

fn update_dotnet_project(path: &Path, new_version: &Version) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let mut root = parse_xml(&content, path)?;
    let version_string = new_version.to_string();
    let mut changed = false;
    update_elements_named(&mut root, "Version", &mut |element| {
        set_element_text(element, &version_string);
        changed = true;
    });

    if !changed {
        return Ok(false);
    }
    write_xml_if_changed(path, &root, &content)
}

fn update_dotnet_assembly_info(path: &Path, new_version: &Version) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let numeric = format!("{}.{}.{}.0", new_version.major, new_version.minor, new_version.patch);
    let informational = new_version.to_string();
    let mut updated = content.clone();
    updated = DOTNET_ASSEMBLY_VERSION_RE.replace_all(&updated, format!("AssemblyVersion(\"{numeric}\")")).into_owned();
    updated = DOTNET_FILE_VERSION_RE.replace_all(&updated, format!("AssemblyFileVersion(\"{numeric}\")")).into_owned();
    updated = DOTNET_INFO_VERSION_RE
        .replace_all(&updated, format!("AssemblyInformationalVersion(\"{informational}\")")).into_owned();
    if updated == content {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn update_ruby_gemspec(path: &Path, new_version: &Version) -> Result<bool> {
    replace_single_capture(path, &RUBY_GEMSPEC_VERSION_RE, &new_version.to_string())
}

fn update_ruby_version_file(path: &Path, new_version: &Version) -> Result<bool> {
    replace_single_capture(path, &RUBY_VERSION_RB_RE, &new_version.to_string())
}

fn update_composer_json(path: &Path, new_version: &Version) -> Result<bool> {
    update_json_version_field(path, new_version, |json| json.get("version").is_some())
}

fn update_xcode_project(path: &Path, new_version: &Version) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let version_string = new_version.to_string();
    let updated = XCODE_MARKETING_VERSION_RE.replace_all(&content, |captures: &Captures<'_>| {
        let whole = captures.get(0).unwrap().as_str();
        let matched = captures.name("value_dq").unwrap().as_str();
        whole.replacen(matched, &version_string, 1)
    }).into_owned();
    if updated == content {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn update_info_plist(path: &Path, new_version: &Version) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let version_string = new_version.to_string();
    let updated = PLIST_BUNDLE_VERSION_RE.replace_all(&content, |captures: &Captures<'_>| {
        let whole = captures.get(0).unwrap().as_str();
        let matched = captures.name("value_dq").unwrap().as_str();
        whole.replacen(matched, &version_string, 1)
    }).into_owned();
    if updated == content {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn update_json_version_field<F>(path: &Path, new_version: &Version, should_manage: F) -> Result<bool> where F: Fn(&JsonMap<String, JsonValue>) -> bool {
    let content = fs::read_to_string(path)?;
    let mut json: JsonValue = serde_json::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(root) = json.as_object_mut() else {
        bail!("{} is not a JSON object", path.display());
    };
    if !should_manage(root) {
        return Ok(false);
    }
    root.insert("version".to_string(), JsonValue::String(new_version.to_string()));
    let updated = serde_json::to_string_pretty(&json)?;
    if updated == content {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn parse_consistent_versions(ecosystem: Ecosystem, path: &Path, target_kind: TargetKind, authority: TargetAuthority, raw_versions: &[String]) -> Result<VersionTarget> {
    parse_consistent_versions_with_dynamic(ecosystem, path, target_kind, authority, raw_versions)
}

fn parse_consistent_versions_with_dynamic(ecosystem: Ecosystem, path: &Path, target_kind: TargetKind, authority: TargetAuthority, raw_versions: &[String]) -> Result<VersionTarget> {
    if raw_versions.iter().any(|value| looks_dynamic(value)) {
        return Ok(VersionTarget::issue(ecosystem, path, target_kind, authority, TargetStatus::SkippedDynamic, raw_versions.first().cloned(), "dynamic version expression is not supported"));
    }

    let parsed = raw_versions
        .iter()
        .map(|value| parse_version(ecosystem, path, target_kind, authority, value))
        .collect::<Result<Vec<_>>>();

    match parsed {
        Ok(parsed) => {
            let mut unique = parsed.iter().map(ToString::to_string).collect::<Vec<_>>();
            unique.sort();
            unique.dedup();
            if unique.len() > 1 {
                return Ok(VersionTarget::issue(ecosystem, path, target_kind, authority, TargetStatus::Error, unique.first().cloned(), format!("file contains conflicting versions: {}", unique.join(", "))));
            }

            Ok(VersionTarget::managed(ecosystem, path, target_kind, authority, parsed[0].clone()))
        }
        Err(error) => Ok(VersionTarget::issue(ecosystem, path, target_kind, authority, TargetStatus::Error, raw_versions.first().cloned(), error.to_string()))
    }
}

fn parse_version(ecosystem: Ecosystem, path: &Path, target_kind: TargetKind, authority: TargetAuthority, raw: &str) -> Result<Version> {
    Version::parse(raw)
        .map_err(|error| {anyhow!("{:?} {:?} in {} has invalid semver `{raw}`: {error}", ecosystem, target_kind, path.display())})
        .map(|version| {
            let _ = authority;
            version
        })
}

fn parse_toml_document(content: &str, path: &Path) -> Result<DocumentMut> {
    content.parse::<DocumentMut>().with_context(|| format!("failed to parse {}", path.display()))
}

fn parse_xml(content: &str, path: &Path) -> Result<Element> {
    Element::parse(content.as_bytes()).with_context(|| format!("failed to parse {}", path.display()))
}

fn get_item<'a>(doc: &'a DocumentMut, path: &[&str]) -> Option<&'a Item> {
    let mut item = doc.as_item();
    for key in path {
        item = item.get(key)?;
    }
    Some(item)
}

fn item_string(item: Option<&Item>) -> Option<String> {
    item.and_then(|item| item.as_value())
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn item_bool(item: Option<&Item>) -> Option<bool> {
    item.and_then(|item| item.as_value())
        .and_then(|value| value.as_bool())
}

fn dynamic_array_contains_version(item: Option<&Item>) -> bool {
    item.and_then(|item| item.as_array()).map(|array| {array.iter().filter_map(|value| value.as_str()).any(|entry| entry == "version")}).unwrap_or(false)
}

fn child_element<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    element.children.iter().find_map(|child| match child {
        XMLNode::Element(child) if local_name(&child.name) == name => Some(child),
        _ => None,
    })
}

fn child_element_mut<'a>(element: &'a mut Element, name: &str) -> Option<&'a mut Element> {
    element.children.iter_mut().find_map(|child| match child {
        XMLNode::Element(child) if local_name(&child.name) == name => Some(child),
        _ => None,
    })
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn set_element_text(element: &mut Element, text: &str) {
    element.children.retain(|child| !matches!(child, XMLNode::Text(_)));
    element.children.push(XMLNode::Text(text.to_string()));
}

fn collect_texts_by_name(element: &Element, name: &str, output: &mut Vec<String>) {
    if local_name(&element.name) == name {
        if let Some(value) = element.get_text() {
            output.push(value.into_owned());
        }
    }

    for child in &element.children {
        if let XMLNode::Element(child) = child {
            collect_texts_by_name(child, name, output);
        }
    }
}

fn update_elements_named(element: &mut Element, name: &str, callback: &mut impl FnMut(&mut Element)) {
    if local_name(&element.name) == name {
        callback(element);
    }

    for child in &mut element.children {
        if let XMLNode::Element(child) = child {
            update_elements_named(child, name, callback);
        }
    }
}

fn resolves_local_maven_parent(repo_root: &Path, relative_path: &Path, parent: &Element) -> bool {
    let relative_parent = child_element(parent, "relativePath")
        .and_then(Element::get_text)
        .map(Cow::into_owned)
        .unwrap_or_else(|| "../pom.xml".to_string());
    let candidate = relative_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(relative_parent);
    let candidate = repo_root.join(candidate);
    candidate.exists() && candidate.starts_with(repo_root)
}

fn ini_value(content: &str, section: &str, key: &str) -> Option<String> {
    let mut current_section = None::<String>;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = Some(trimmed[1..trimmed.len() - 1].trim().to_string());
            continue;
        }
        if current_section.as_deref() != Some(section) {
            continue;
        }
        if let Some((candidate_key, value)) = line.split_once('=') {
            if candidate_key.trim() == key {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

fn ini_value_span(content: &str, section: &str, key: &str) -> Option<(usize, usize, String)> {
    let mut current_section = None::<String>;
    let mut offset = 0usize;
    for line in content.lines() {
        let line_len = line.len();
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = Some(trimmed[1..trimmed.len() - 1].trim().to_string());
        } else if current_section.as_deref() == Some(section) {
            if let Some((candidate_key, _)) = line.split_once('=') {
                if candidate_key.trim() == key {
                    let prefix = format!("{} = ", candidate_key.trim());
                    return Some((offset, offset + line_len, prefix));
                }
            }
        }
        offset += line_len + 1;
    }
    None
}

fn normalize_extra_newline(content: String) -> String {
    content.replace("\n\n\n", "\n\n")
}

fn capture_regex_value<'a>(captures: &'a Captures<'_>) -> Option<&'a str> {
    captures
        .name("value_dq")
        .or_else(|| captures.name("value_sq"))
        .map(|value| value.as_str())
}

fn replace_single_capture(path: &Path, regex: &Regex, value: &str) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let mut count = 0usize;
    let updated = regex
        .replace_all(&content, |captures: &Captures<'_>| {
            count += 1;
            let whole = captures.get(0).unwrap().as_str();
            let matched = capture_regex_value(captures).unwrap();
            whole.replacen(matched, value, 1)
        })
        .into_owned();
    if count == 0 || updated == content {
        return Ok(false);
    }
    if count > 1 {
        bail!("{} had multiple matching assignments", path.display());
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn write_xml_if_changed(path: &Path, root: &Element, original: &str) -> Result<bool> {
    let mut buffer = Vec::new();
    root.write(&mut buffer).with_context(|| format!("failed to serialize {}", path.display()))?;
    let updated = String::from_utf8(buffer).context("xml serializer produced invalid UTF-8")?;
    if updated == original {
        return Ok(false);
    }
    fs::write(path, updated)?;
    Ok(true)
}

fn looks_dynamic(value: &str) -> bool {
    value.contains("${")
        || value.contains("$(")
        || value.contains("%(")
        || value.contains("@(")
        || value.contains("{version}")
}

fn should_skip_path(relative_path: &Path) -> bool {
    relative_path.components().any(|component| {
        matches!(component, Component::Normal(name) if SKIP_DIRS.iter().any(|candidate| name == OsStr::new(candidate)))
    })
}

fn is_excluded(relative_path: &Path, excludes: &[String]) -> bool {
    let relative = path_display(relative_path);
    excludes.iter().any(|exclude| {
        let trimmed = exclude.trim().trim_matches('/');
        let trimmed = trimmed.trim_end_matches("/**").trim_end_matches("/*").trim_end_matches('*');
        let trimmed = trimmed.trim_end_matches('/');
        !trimmed.is_empty() && (relative == trimmed || relative.starts_with(&format!("{trimmed}/")))
    })
}

fn path_contains_component(path: &Path, component_name: &str) -> bool {
    path.components()
        .any(|component| matches!(component, Component::Normal(name) if name == OsStr::new(component_name)))
}

fn path_display(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_dotnet_numeric_version(value: &str) -> Option<String> {
    let segments = value.split('.').collect::<Vec<_>>();
    match segments.as_slice() {
        [major, minor, patch] => Some(format!("{major}.{minor}.{patch}")),
        [major, minor, patch, "0"] => Some(format!("{major}.{minor}.{patch}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_commit_prefixes() {
        assert_eq!(extract_commit_prefix("fix: bug"), Some("fix".to_string()));
        assert_eq!(extract_commit_prefix("feature(api): add route"), Some("feature".to_string()));
        assert_eq!(extract_commit_prefix("docs update"), None);
    }

    #[test]
    fn normalizes_dotnet_numeric_versions() {
        assert_eq!(normalize_dotnet_numeric_version("1.2.3.0"), Some("1.2.3".to_string()));
        assert_eq!(normalize_dotnet_numeric_version("1.2.3"), Some("1.2.3".to_string()));
        assert_eq!(normalize_dotnet_numeric_version("1.2"), None);
    }

    #[test]
    fn excludes_nested_paths() {
        assert!(is_excluded(Path::new("packages/api/package.json"), &[String::from("packages/api")]));
        assert!(!is_excluded(Path::new("packages/web/package.json"), &[String::from("packages/api")]));
    }

    #[test]
    fn excludes_glob_patterns() {
        assert!(is_excluded(Path::new("test/extensions/file.csproj"), &[String::from("test/**")]));
        assert!(is_excluded(Path::new("test/filenames/pom.xml"), &[String::from("test/**")]));
        assert!(is_excluded(Path::new("test/deep/nested/file.json"), &[String::from("test/*")]));
        assert!(!is_excluded(Path::new("src/package.json"), &[String::from("test/**")]));
    }
}
