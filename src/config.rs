use std::{collections::BTreeMap, fmt, fs, path::Path};

use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum BumpLevel {
    Patch,
    Minor,
    Major,
}

impl fmt::Display for BumpLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    All,
    Branch,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::All => "all",
            Self::Branch => "branch",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum PromptMode {
    Never,
    Always,
    Ask,
}

impl fmt::Display for PromptMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Never => "never",
            Self::Always => "always",
            Self::Ask => "ask",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MonorepoMode {
    Lockstep,
}

impl fmt::Display for MonorepoMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Lockstep => "lockstep",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConstantsMode {
    Curated,
}

impl fmt::Display for ConstantsMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Curated => "curated",
        };
        f.write_str(label)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub on_bump: Vec<BumpLevel>,
    #[serde(default, rename = "match")]
    pub r#match: String,
    #[serde(default)]
    pub skip: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_mode")]
    pub mode: Mode,
    #[serde(default = "default_prompt")]
    pub prompt: PromptMode,
    #[serde(default = "default_prompt_prefixes")]
    pub prompt_prefixes: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            prompt: default_prompt(),
            prompt_prefixes: default_prompt_prefixes(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSettings {
    #[serde(default = "default_monorepo_mode")]
    pub monorepo: MonorepoMode,
    #[serde(default = "default_constants_mode")]
    pub constants: ConstantsMode,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            monorepo: default_monorepo_mode(),
            constants: default_constants_mode(),
            exclude: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SsmverConfig {
    #[serde(default = "default_version")]
    pub version: Version,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub sync: SyncSettings,
    #[serde(default = "default_prefixes")]
    pub prefixes: BTreeMap<String, BumpLevel>,
    #[serde(default)]
    pub release: ReleaseConfig,
}

impl Default for SsmverConfig {
    fn default() -> Self {
        Self {
            version: default_version(),
            settings: Settings::default(),
            sync: SyncSettings::default(),
            prefixes: default_prefixes(),
            release: ReleaseConfig::default(),
        }
    }
}

impl SsmverConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Self::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn from_str(raw: &str) -> Result<Self> {
        Ok(toml::from_str(raw)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let raw = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(path, raw).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn get_config_value(&self, key: &str) -> Result<String> {
        match normalize_key(key) {
            "version" => Ok(self.version.to_string()),
            "mode" => Ok(self.settings.mode.to_string()),
            "prompt" => Ok(self.settings.prompt.to_string()),
            "prompt_prefixes" => Ok(self.settings.prompt_prefixes.join(",")),
            "sync.monorepo" => Ok(self.sync.monorepo.to_string()),
            "sync.constants" => Ok(self.sync.constants.to_string()),
            "sync.exclude" => Ok(self.sync.exclude.join(",")),
            "release.enabled" => Ok(self.release.enabled.to_string()),
            "release.on_bump" => Ok(self.release.on_bump.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(",")),
            "release.match" => Ok(self.release.r#match.clone()),
            "release.skip" => Ok(self.release.skip.clone()),
            _ => bail!("Unsupported config key: {key}"),
        }
    }

    pub fn set_config_value(&mut self, key: &str, raw_value: &str) -> Result<String> {
        match normalize_key(key) {
            "version" => {
                self.version = raw_value.parse()?;
                Ok(format!("Set version = \"{}\"", self.version))
            }
            "mode" => {
                self.settings.mode = parse_mode(raw_value)?;
                Ok(format!("Set mode = \"{}\"", self.settings.mode))
            }
            "prompt" => {
                self.settings.prompt = parse_prompt_mode(raw_value)?;
                Ok(format!("Set prompt = \"{}\"", self.settings.prompt))
            }
            "prompt_prefixes" => {
                self.settings.prompt_prefixes = parse_string_list(raw_value);
                Ok(format!(
                    "Set prompt_prefixes = {}",
                    render_string_array(&self.settings.prompt_prefixes)
                ))
            }
            "sync.monorepo" => {
                self.sync.monorepo = parse_monorepo_mode(raw_value)?;
                Ok(format!("Set sync.monorepo = \"{}\"", self.sync.monorepo))
            }
            "sync.constants" => {
                self.sync.constants = parse_constants_mode(raw_value)?;
                Ok(format!("Set sync.constants = \"{}\"", self.sync.constants))
            }
            "sync.exclude" => {
                self.sync.exclude = parse_string_list(raw_value);
                Ok(format!(
                    "Set sync.exclude = {}",
                    render_string_array(&self.sync.exclude)
                ))
            }
            "release.enabled" => {
                self.release.enabled = parse_bool(raw_value)?;
                Ok(format!("Set release.enabled = {}", self.release.enabled))
            }
            "release.on_bump" => {
                self.release.on_bump = parse_bump_level_list(raw_value)?;
                let display = self.release.on_bump.iter().map(|l| l.to_string()).collect::<Vec<_>>();
                Ok(format!("Set release.on_bump = {}", render_string_array(&display)))
            }
            "release.match" => {
                self.release.r#match = raw_value.trim().to_string();
                Ok(format!("Set release.match = \"{}\"", self.release.r#match))
            }
            "release.skip" => {
                self.release.skip = raw_value.trim().to_string();
                Ok(format!("Set release.skip = \"{}\"", self.release.skip))
            }
            _ => bail!("Unsupported config key: {key}"),
        }
    }
}

pub fn compute_next_version(current: &Version, level: BumpLevel) -> Version {
    let mut next = current.clone();
    match level {
        BumpLevel::Patch => {
            next.patch += 1;
        }
        BumpLevel::Minor => {
            next.minor += 1;
            next.patch = 0;
        }
        BumpLevel::Major => {
            next.major += 1;
            next.minor = 0;
            next.patch = 0;
        }
    }
    next.pre = semver::Prerelease::EMPTY;
    next.build = semver::BuildMetadata::EMPTY;
    next
}

fn normalize_key(key: &str) -> &str {
    match key {
        "settings.mode" => "mode",
        "settings.prompt" => "prompt",
        "settings.prompt_prefixes" => "prompt_prefixes",
        "sync.monorepo" => "sync.monorepo",
        "sync.constants" => "sync.constants",
        "sync.exclude" => "sync.exclude",
        "monorepo" => "sync.monorepo",
        "constants" => "sync.constants",
        "exclude" => "sync.exclude",
        "enabled" => "release.enabled",
        "on_bump" => "release.on_bump",
        "release.match" => "release.match",
        "release.skip" => "release.skip",
        "release.enabled" => "release.enabled",
        "release.on_bump" => "release.on_bump",
        other => other,
    }
}

fn parse_mode(value: &str) -> Result<Mode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "all" => Ok(Mode::All),
        "branch" => Ok(Mode::Branch),
        _ => bail!("mode must be one of: all, branch"),
    }
}

fn parse_prompt_mode(value: &str) -> Result<PromptMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "never" => Ok(PromptMode::Never),
        "always" => Ok(PromptMode::Always),
        "ask" => Ok(PromptMode::Ask),
        _ => bail!("prompt must be one of: never, always, ask"),
    }
}

fn parse_monorepo_mode(value: &str) -> Result<MonorepoMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "lockstep" => Ok(MonorepoMode::Lockstep),
        _ => bail!("sync.monorepo must be: lockstep"),
    }
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => bail!("value must be true or false"),
    }
}

fn parse_bump_level_list(value: &str) -> Result<Vec<BumpLevel>> {
    let items = parse_string_list(value);
    items.iter().map(|item| match item.trim().to_ascii_lowercase().as_str() {
        "patch" => Ok(BumpLevel::Patch),
        "minor" => Ok(BumpLevel::Minor),
        "major" => Ok(BumpLevel::Major),
        other => bail!("unknown bump level: {other} (expected patch, minor, or major)"),
    }).collect()
}

fn parse_constants_mode(value: &str) -> Result<ConstantsMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "curated" => Ok(ConstantsMode::Curated),
        _ => bail!("sync.constants must be: curated"),
    }
}

pub fn parse_string_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|entry| entry.trim().trim_matches('"').trim_matches('\''))
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.to_string())
        .collect()
}

fn render_string_array(values: &[String]) -> String {
    let body = values
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{body}]")
}

fn default_version() -> Version {
    Version::new(0, 1, 0)
}

fn default_mode() -> Mode {
    Mode::Branch
}

fn default_prompt() -> PromptMode {
    PromptMode::Never
}

fn default_prompt_prefixes() -> Vec<String> {
    vec!["feature".to_string(), "release".to_string()]
}

fn default_monorepo_mode() -> MonorepoMode {
    MonorepoMode::Lockstep
}

fn default_constants_mode() -> ConstantsMode {
    ConstantsMode::Curated
}

fn default_prefixes() -> BTreeMap<String, BumpLevel> {
    BTreeMap::from([
        ("feature".to_string(), BumpLevel::Minor),
        ("fix".to_string(), BumpLevel::Patch),
        ("release".to_string(), BumpLevel::Major),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_bumps_reset_lower_segments_and_metadata() {
        let version = Version::parse("1.2.3-beta.4+sha").unwrap();
        assert_eq!(
            compute_next_version(&version, BumpLevel::Patch).to_string(),
            "1.2.4"
        );
        assert_eq!(
            compute_next_version(&version, BumpLevel::Minor).to_string(),
            "1.3.0"
        );
        assert_eq!(
            compute_next_version(&version, BumpLevel::Major).to_string(),
            "2.0.0"
        );
    }

    #[test]
    fn string_lists_accept_csv_or_array_shapes() {
        assert_eq!(
            parse_string_list("feature, release"),
            vec!["feature".to_string(), "release".to_string()]
        );
        assert_eq!(
            parse_string_list("[\"feature\", \"release\"]"),
            vec!["feature".to_string(), "release".to_string()]
        );
    }

    #[test]
    fn config_defaults_include_sync_settings() {
        let config = SsmverConfig::default();
        assert_eq!(config.version, Version::new(0, 1, 0));
        assert_eq!(config.sync.exclude, Vec::<String>::new());
        assert_eq!(config.sync.monorepo, MonorepoMode::Lockstep);
        assert_eq!(config.sync.constants, ConstantsMode::Curated);
    }
}
