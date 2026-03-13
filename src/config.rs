use std::{collections::BTreeMap, fmt, fs, path::Path, str::FromStr};

use anyhow::{anyhow, bail, Context, Result};
use clap::ValueEnum;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn bump(&self, level: BumpLevel) -> Self {
        match level {
            BumpLevel::Patch => Self::new(self.major, self.minor, self.patch + 1),
            BumpLevel::Minor => Self::new(self.major, self.minor + 1, 0),
            BumpLevel::Major => Self::new(self.major + 1, 0, 0),
        }
    }
}

impl Default for Version {
    fn default() -> Self {
        Self::new(0, 1, 0)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for Version {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let mut segments = value.split('.');
        let major = segments
            .next()
            .ok_or_else(|| anyhow!("missing major version"))?
            .parse()
            .context("invalid major version")?;
        let minor = segments
            .next()
            .ok_or_else(|| anyhow!("missing minor version"))?
            .parse()
            .context("invalid minor version")?;
        let patch = segments
            .next()
            .ok_or_else(|| anyhow!("missing patch version"))?
            .parse()
            .context("invalid patch version")?;

        if segments.next().is_some() {
            bail!("version must be in major.minor.patch format");
        }

        Ok(Self::new(major, minor, patch))
    }
}

impl Serialize for Version {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(de::Error::custom)
    }
}

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
pub struct SsmverConfig {
    #[serde(default)]
    pub version: Version,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default = "default_prefixes")]
    pub prefixes: BTreeMap<String, BumpLevel>,
}

impl Default for SsmverConfig {
    fn default() -> Self {
        Self {
            version: Version::default(),
            settings: Settings::default(),
            prefixes: default_prefixes(),
        }
    }
}

impl SsmverConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: Self =
            toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let raw = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(path, raw).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn bump_version(&mut self, level: BumpLevel) -> (Version, Version) {
        let before = self.version.clone();
        let after = before.bump(level);
        self.version = after.clone();
        (before, after)
    }

    pub fn get_config_value(&self, key: &str) -> Result<String> {
        match normalize_key(key) {
            "version" => Ok(self.version.to_string()),
            "mode" => Ok(self.settings.mode.to_string()),
            "prompt" => Ok(self.settings.prompt.to_string()),
            "prompt_prefixes" => Ok(self.settings.prompt_prefixes.join(",")),
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
                self.settings.prompt_prefixes = parse_prompt_prefixes(raw_value);
                Ok(format!(
                    "Set prompt_prefixes = {}",
                    render_string_array(&self.settings.prompt_prefixes)
                ))
            }
            _ => bail!("Unsupported config key: {key}"),
        }
    }
}

fn normalize_key(key: &str) -> &str {
    match key {
        "settings.mode" => "mode",
        "settings.prompt" => "prompt",
        "settings.prompt_prefixes" => "prompt_prefixes",
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

fn parse_prompt_prefixes(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|entry| entry.trim().trim_matches('"'))
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

fn default_mode() -> Mode {
    Mode::Branch
}

fn default_prompt() -> PromptMode {
    PromptMode::Never
}

fn default_prompt_prefixes() -> Vec<String> {
    vec!["feature".to_string(), "release".to_string()]
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
    fn version_bumps_reset_lower_segments() {
        let version = Version::new(1, 2, 3);
        assert_eq!(version.bump(BumpLevel::Patch).to_string(), "1.2.4");
        assert_eq!(version.bump(BumpLevel::Minor).to_string(), "1.3.0");
        assert_eq!(version.bump(BumpLevel::Major).to_string(), "2.0.0");
    }

    #[test]
    fn version_parsing_rejects_invalid_shapes() {
        assert!("1.2".parse::<Version>().is_err());
        assert!("1.2.3.4".parse::<Version>().is_err());
        assert!("foo.bar.baz".parse::<Version>().is_err());
    }

    #[test]
    fn prompt_prefixes_accept_csv_or_toml_array() {
        assert_eq!(
            parse_prompt_prefixes("feature, release"),
            vec!["feature".to_string(), "release".to_string()]
        );
        assert_eq!(
            parse_prompt_prefixes("[\"feature\", \"release\"]"),
            vec!["feature".to_string(), "release".to_string()]
        );
    }
}
