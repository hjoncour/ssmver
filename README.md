# ssmver

`ssmver` is a Rust CLI that keeps a repository version in sync across common project manifests and build files.

It stores the canonical version in `ssmver.toml`, installs git hooks, and bumps that version automatically when commit messages use configured prefixes such as:

- `fix: correct null handling` -> patch bump
- `feature(auth): add OAuth login` -> minor bump
- `release(v2): break old API` -> major bump

After a bump, `ssmver` propagates the new version to supported project files in the repo instead of only updating `ssmver.toml`.

## What it supports

Current support includes:

- Rust / Cargo: `Cargo.toml`, including workspace package versions
- Node / JS / TS: `package.json`, `package-lock.json`, `npm-shrinkwrap.json`
- Maven: `pom.xml` project version and local parent version references
- Gradle: `gradle.properties`, simple literal `build.gradle` / `build.gradle.kts` version assignments
- Python: `pyproject.toml`, `setup.cfg`, simple `setup.py`, curated `__version__` constants
- .NET: `*.csproj`, `Directory.Build.props`, curated `AssemblyInfo.cs` version attributes
- Ruby: `*.gemspec`, curated `lib/**/version.rb`
- PHP: `composer.json`

`ssmver` intentionally does not try to replace arbitrary version strings in docs, source comments, changelogs, or dependency declarations.

## How it works

Running `ssmver init` in a git repository will:

1. Create `.ssmver/hooks/`
2. Generate git hook wrapper scripts
3. Set `git config core.hooksPath .ssmver/hooks`
4. Add `.ssmver/` to `.gitignore`
5. Create `ssmver.toml` if it does not already exist
6. Import the current version from supported project files when they agree

The installed hooks call the `ssmver` binary, so `ssmver` should be installed on each machine that makes commits in the repo.
If you later upgrade the `ssmver` binary, run `ssmver update` in a managed repo to refresh its hook wrappers and re-sync versioned files.

Default config:

```toml
version = "0.1.0"

[settings]
mode = "branch"
prompt = "never"
prompt_prefixes = ["feature", "release"]

[sync]
monorepo = "lockstep"
constants = "curated"
exclude = []

[prefixes]
fix = "patch"
feature = "minor"
release = "major"
```

## Installation

Install with Cargo.

### Install from a local checkout

```bash
git clone https://github.com/hjoncour/ssmver.git
cd ssmver
cargo install --path .
```

To rebuild and reinstall the current checkout after editing `ssmver` itself:

```bash
make update
```

### Install directly from GitHub

```bash
cargo install --git https://github.com/hjoncour/ssmver.git
```

### Development usage without installing

```bash
cargo run -- init
```

## Quick start

```bash
cd /path/to/your/project
ssmver init
ssmver targets list
git commit -m "fix: handle empty response"
```

Useful commands:

```bash
ssmver prefix add hotfix patch
ssmver prefix remove hotfix
ssmver prefix list
ssmver bump minor
ssmver update
ssmver version
ssmver config mode all
ssmver targets list --json
ssmver uninstall
```

## Notes

- `ssmver.toml` is meant to be committed so the team shares the same versioning rules.
- `.ssmver/` is generated and should stay ignored.
- Running `ssmver init` again is safe and refreshes the installed hook wrappers.
- If `ssmver` finds a supported file with a dynamic or ambiguous version expression, it will refuse to sync until you exclude or simplify that file.
