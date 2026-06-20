use crate::config::{BumpLevel, ReleaseConfig};
use crate::targets::Ecosystem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowKind {
    Release,
    Package,
    Npm,
    Crates,
    Marketplace,
}

pub fn workflow_file_name(kind: WorkflowKind) -> &'static str {
    match kind {
        WorkflowKind::Release => "release.yaml",
        WorkflowKind::Package => "package.yaml",
        WorkflowKind::Npm => "npm.yaml",
        WorkflowKind::Crates => "crates.yaml",
        WorkflowKind::Marketplace => "marketplace.yaml",
    }
}

pub fn release_workflow(config: &ReleaseConfig, branch: &str) -> String {
    let trigger_step = trigger_evaluation_step(config);
    r#"name: release

on:
  push:
    branches: [__BRANCH__]

permissions:
  contents: write

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

__TRIGGER_STEP__

      - name: Create GitHub Release
        if: steps.check.outputs.should_release == 'true'
        uses: softprops/action-gh-release@v2
        with:
          tag_name: v${{ steps.check.outputs.version }}
          name: v${{ steps.check.outputs.version }}
          generate_release_notes: true
"#
    .replace("__BRANCH__", branch)
    .replace("__TRIGGER_STEP__", &trigger_step)
}

pub fn npm_workflow(config: &ReleaseConfig, branch: &str) -> String {
    let trigger_step = trigger_evaluation_step(config);
    r#"name: npm

on:
  push:
    branches: [__BRANCH__]

permissions:
  contents: read

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

__TRIGGER_STEP__

      - uses: actions/setup-node@v4
        if: steps.check.outputs.should_release == 'true'
        with:
          node-version: lts/*
          registry-url: https://registry.npmjs.org

      - name: Install dependencies
        if: steps.check.outputs.should_release == 'true'
        run: npm ci

      - name: Publish to npm
        if: steps.check.outputs.should_release == 'true'
        run: npm publish
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
"#
    .replace("__BRANCH__", branch)
    .replace("__TRIGGER_STEP__", &trigger_step)
}

pub fn crates_workflow(config: &ReleaseConfig, branch: &str) -> String {
    let trigger_step = trigger_evaluation_step(config);
    r#"name: crates

on:
  push:
    branches: [__BRANCH__]

permissions:
  contents: read

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

__TRIGGER_STEP__

      - uses: dtolnay/rust-toolchain@stable
        if: steps.check.outputs.should_release == 'true'

      - name: Publish to crates.io
        if: steps.check.outputs.should_release == 'true'
        run: cargo publish
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
"#
    .replace("__BRANCH__", branch)
    .replace("__TRIGGER_STEP__", &trigger_step)
}

pub fn marketplace_workflow(config: &ReleaseConfig, branch: &str) -> String {
    let trigger_step = trigger_evaluation_step(config);
    r#"name: marketplace

on:
  push:
    branches: [__BRANCH__]

permissions:
  contents: read

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

__TRIGGER_STEP__

      - uses: actions/setup-node@v4
        if: steps.check.outputs.should_release == 'true'
        with:
          node-version: lts/*

      - name: Install dependencies
        if: steps.check.outputs.should_release == 'true'
        run: npm ci

      - name: Package extension
        if: steps.check.outputs.should_release == 'true'
        run: npx @vscode/vsce package

      - name: Publish to VS Marketplace
        if: steps.check.outputs.should_release == 'true'
        run: npx @vscode/vsce publish --packagePath *.vsix
        env:
          VSCE_PAT: ${{ secrets.VSCE_PAT }}

      - name: Publish to Open VSX
        if: steps.check.outputs.should_release == 'true'
        run: npx ovsx publish *.vsix -p ${{ secrets.OVSX_TOKEN }}
"#
    .replace("__BRANCH__", branch)
    .replace("__TRIGGER_STEP__", &trigger_step)
}

pub fn package_workflow(config: &ReleaseConfig, branch: &str, ecosystem: Ecosystem) -> String {
    let trigger_step = trigger_evaluation_step(config);
    let publish_steps = match ecosystem {
        Ecosystem::Node => {
            r#"      - uses: actions/setup-node@v4
        if: steps.check.outputs.should_release == 'true'
        with:
          node-version: lts/*
          registry-url: https://npm.pkg.github.com

      - name: Install dependencies
        if: steps.check.outputs.should_release == 'true'
        run: npm ci

      - name: Publish to GitHub Packages
        if: steps.check.outputs.should_release == 'true'
        run: npm publish
        env:
          NODE_AUTH_TOKEN: ${{ secrets.GITHUB_TOKEN }}"#
        }

        Ecosystem::Maven => {
            r#"      - uses: actions/setup-java@v4
        if: steps.check.outputs.should_release == 'true'
        with:
          java-version: '17'
          distribution: temurin

      - name: Publish to GitHub Packages
        if: steps.check.outputs.should_release == 'true'
        run: mvn deploy -DskipTests
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}"#
        }

        Ecosystem::Gradle => {
            r#"      - uses: actions/setup-java@v4
        if: steps.check.outputs.should_release == 'true'
        with:
          java-version: '17'
          distribution: temurin

      - name: Publish to GitHub Packages
        if: steps.check.outputs.should_release == 'true'
        run: ./gradlew publish
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}"#
        }

        Ecosystem::Dotnet => {
            r#"      - uses: actions/setup-dotnet@v4
        if: steps.check.outputs.should_release == 'true'

      - name: Pack and publish to GitHub Packages
        if: steps.check.outputs.should_release == 'true'
        run: |
          dotnet pack --configuration Release
          dotnet nuget push **/*.nupkg --source "https://nuget.pkg.github.com/${{ github.repository_owner }}/index.json" --api-key ${{ secrets.GITHUB_TOKEN }}"#
        }

        Ecosystem::Ruby => {
            r#"      - uses: ruby/setup-ruby@v1
        if: steps.check.outputs.should_release == 'true'
        with:
          ruby-version: '3.2'

      - name: Publish to GitHub Packages
        if: steps.check.outputs.should_release == 'true'
        run: |
          gem build *.gemspec
          gem push --host https://rubygems.pkg.github.com/${{ github.repository_owner }} *.gem
        env:
          GEM_HOST_API_KEY: ${{ secrets.GITHUB_TOKEN }}"#
        }

        _ => "",
    };

    let template = r#"name: package

on:
  push:
    branches: [__BRANCH__]

permissions:
  contents: read
  packages: write

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

__TRIGGER_STEP__

__PUBLISH_STEPS__
"#;

    template
        .replace("__BRANCH__", branch)
        .replace("__TRIGGER_STEP__", &trigger_step)
        .replace("__PUBLISH_STEPS__", publish_steps)
}

fn trigger_evaluation_step(config: &ReleaseConfig) -> String {
    let on_bump_filter = if config.on_bump.is_empty() {
        String::new()
    } else {
        let levels: Vec<&str> = config
            .on_bump
            .iter()
            .map(|l| match l {
                BumpLevel::Patch => "patch",
                BumpLevel::Minor => "minor",
                BumpLevel::Major => "major",
            })
            .collect();
        format!(
            r#"
    # on_bump filter
    ALLOWED_BUMPS="{}"
    if ! echo "$ALLOWED_BUMPS" | grep -qw "$BUMP_TYPE"; then
      echo "Bump type '$BUMP_TYPE' not in allowed list: $ALLOWED_BUMPS"
      echo "should_release=false" >> "$GITHUB_OUTPUT"
      exit 0
    fi"#,
            levels.join(" ")
        )
    };

    let match_filter = if config.r#match.is_empty() {
        String::new()
    } else {
        format!(
            r#"
    # match filter
    MATCH_PATTERN="{}"
    COMMIT_MSG=$(git log -1 --format=%B)
    if ! echo "$COMMIT_MSG" | grep -qF "$MATCH_PATTERN"; then
      echo "Commit message does not contain required pattern: $MATCH_PATTERN"
      echo "should_release=false" >> "$GITHUB_OUTPUT"
      exit 0
    fi"#,
            config.r#match.replace('"', r#"\""#)
        )
    };

    let skip_filter = if config.skip.is_empty() {
        String::new()
    } else {
        format!(
            r#"
    # skip filter
    SKIP_PATTERN="{}"
    COMMIT_MSG=${{COMMIT_MSG:-$(git log -1 --format=%B)}}
    if echo "$COMMIT_MSG" | grep -qF "$SKIP_PATTERN"; then
      echo "Commit message contains skip pattern: $SKIP_PATTERN"
      echo "should_release=false" >> "$GITHUB_OUTPUT"
      exit 0
    fi"#,
            config.skip.replace('"', r#"\""#)
        )
    };

    format!(
        r#"      - name: Evaluate release conditions
        id: check
        run: |
          VERSION=$(grep '^version' ssmver.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
          echo "version=$VERSION" >> "$GITHUB_OUTPUT"

          # Compare against the branch tip before this push so merged PRs with
          # multiple commits still release correctly.
          BEFORE_SHA="${{ github.event.before }}"
          if [ -n "$BEFORE_SHA" ] && [ "$BEFORE_SHA" != "0000000000000000000000000000000000000000" ]; then
            PREV_VERSION=$(git show "$BEFORE_SHA:ssmver.toml" 2>/dev/null | grep '^version' | head -1 | sed 's/.*"\(.*\)"/\1/' || echo "")
          else
            PREV_VERSION=""
          fi

          if [ -z "$PREV_VERSION" ]; then
            echo "No previous ssmver.toml version found; treating this as the first release"
            PREV_VERSION="0.0.0"
          elif [ "$PREV_VERSION" = "$VERSION" ]; then
            echo "No version change detected"
            echo "should_release=false" >> "$GITHUB_OUTPUT"
            exit 0
          fi

          CUR_MAJOR=$(echo "$VERSION" | cut -d. -f1)
          CUR_MINOR=$(echo "$VERSION" | cut -d. -f2)
          PRV_MAJOR=$(echo "$PREV_VERSION" | cut -d. -f1)
          PRV_MINOR=$(echo "$PREV_VERSION" | cut -d. -f2)

          if [ "$CUR_MAJOR" != "$PRV_MAJOR" ]; then
            BUMP_TYPE="major"
          elif [ "$CUR_MINOR" != "$PRV_MINOR" ]; then
            BUMP_TYPE="minor"
          else
            BUMP_TYPE="patch"
          fi
          echo "bump_type=$BUMP_TYPE" >> "$GITHUB_OUTPUT"
{on_bump}{match_filter}{skip_filter}

          echo "should_release=true" >> "$GITHUB_OUTPUT""#,
        on_bump = on_bump_filter,
        match_filter = match_filter,
        skip_filter = skip_filter,
    )
}

pub const GITHUB_PACKAGES_ECOSYSTEMS: &[Ecosystem] = &[
    Ecosystem::Node,
    Ecosystem::Maven,
    Ecosystem::Gradle,
    Ecosystem::Dotnet,
    Ecosystem::Ruby,
];

/// Extracts the branch name from a `find_main_ref` result like `refs/heads/main`.
pub fn branch_name_from_ref(reference: &str) -> &str {
    reference
        .strip_prefix("refs/heads/")
        .or_else(|| reference.strip_prefix("refs/remotes/origin/"))
        .unwrap_or(reference)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_file_names() {
        assert_eq!(workflow_file_name(WorkflowKind::Release), "release.yaml");
        assert_eq!(workflow_file_name(WorkflowKind::Package), "package.yaml");
        assert_eq!(workflow_file_name(WorkflowKind::Npm), "npm.yaml");
        assert_eq!(workflow_file_name(WorkflowKind::Crates), "crates.yaml");
        assert_eq!(
            workflow_file_name(WorkflowKind::Marketplace),
            "marketplace.yaml"
        );
    }

    #[test]
    fn test_release_workflow_contains_trigger_step() {
        let config = ReleaseConfig::default();
        let yaml = release_workflow(&config, "main");
        assert!(yaml.contains("Evaluate release conditions"));
        assert!(yaml.contains("softprops/action-gh-release@v2"));
        assert!(yaml.contains("branches: [main]"));
        assert!(yaml.contains("fetch-depth: 0"));
        assert!(yaml.contains("github.event.before"));
        assert!(yaml.contains("treating this as the first release"));
    }

    #[test]
    fn test_release_workflow_uses_master_branch() {
        let config = ReleaseConfig::default();
        let yaml = release_workflow(&config, "master");
        assert!(yaml.contains("branches: [master]"));
        assert!(!yaml.contains("branches: [main]"));
    }

    #[test]
    fn test_on_bump_filter_baked_into_yaml() {
        let config = ReleaseConfig {
            enabled: true,
            on_bump: vec![BumpLevel::Minor, BumpLevel::Major],
            r#match: String::new(),
            skip: String::new(),
        };
        let yaml = release_workflow(&config, "main");
        assert!(yaml.contains("ALLOWED_BUMPS=\"minor major\""));
    }

    #[test]
    fn test_match_filter_baked_into_yaml() {
        let config = ReleaseConfig {
            enabled: true,
            on_bump: vec![],
            r#match: "[release]".to_string(),
            skip: String::new(),
        };
        let yaml = release_workflow(&config, "main");
        assert!(yaml.contains("MATCH_PATTERN=\"[release]\""));
    }

    #[test]
    fn test_skip_filter_baked_into_yaml() {
        let config = ReleaseConfig {
            enabled: true,
            on_bump: vec![],
            r#match: String::new(),
            skip: "[skip-release]".to_string(),
        };
        let yaml = release_workflow(&config, "main");
        assert!(yaml.contains("SKIP_PATTERN=\"[skip-release]\""));
    }

    #[test]
    fn test_npm_workflow_structure() {
        let config = ReleaseConfig::default();
        let yaml = npm_workflow(&config, "main");
        assert!(yaml.contains("actions/setup-node@v4"));
        assert!(yaml.contains("npm publish"));
        assert!(yaml.contains("NPM_TOKEN"));
    }

    #[test]
    fn test_crates_workflow_structure() {
        let config = ReleaseConfig::default();
        let yaml = crates_workflow(&config, "main");
        assert!(yaml.contains("dtolnay/rust-toolchain@stable"));
        assert!(yaml.contains("cargo publish"));
        assert!(yaml.contains("CARGO_REGISTRY_TOKEN"));
    }

    #[test]
    fn test_package_workflow_node() {
        let config = ReleaseConfig::default();
        let yaml = package_workflow(&config, "main", Ecosystem::Node);
        assert!(yaml.contains("npm.pkg.github.com"));
        assert!(yaml.contains("packages: write"));
    }

    #[test]
    fn test_package_workflow_dotnet() {
        let config = ReleaseConfig::default();
        let yaml = package_workflow(&config, "main", Ecosystem::Dotnet);
        assert!(yaml.contains("nuget.pkg.github.com"));
        assert!(yaml.contains("dotnet pack"));
    }

    #[test]
    fn test_marketplace_workflow_structure() {
        let config = ReleaseConfig::default();
        let yaml = marketplace_workflow(&config, "main");
        assert!(yaml.contains("actions/setup-node@v4"));
        assert!(yaml.contains("@vscode/vsce package"));
        assert!(yaml.contains("@vscode/vsce publish"));
        assert!(yaml.contains("VSCE_PAT"));
        assert!(yaml.contains("ovsx publish"));
        assert!(yaml.contains("OVSX_TOKEN"));
        assert!(yaml.contains("branches: [main]"));
    }

    #[test]
    fn test_branch_name_from_ref() {
        assert_eq!(branch_name_from_ref("refs/heads/main"), "main");
        assert_eq!(branch_name_from_ref("refs/heads/master"), "master");
        assert_eq!(branch_name_from_ref("refs/remotes/origin/main"), "main");
        assert_eq!(branch_name_from_ref("develop"), "develop");
    }
}
