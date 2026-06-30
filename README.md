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
- Tauri: `tauri.conf.json` app versions
- Maven: `pom.xml` project version and local parent version references
- Gradle: `gradle.properties`, simple literal `build.gradle` / `build.gradle.kts` version assignments
- Python: `pyproject.toml`, `setup.cfg`, simple `setup.py`, curated `__version__` constants
- .NET: `*.csproj`, `Directory.Build.props`, curated `AssemblyInfo.cs` version attributes
- Ruby: `*.gemspec`, curated `lib/**/version.rb`
- PHP: `composer.json`
- Swift / Xcode: `MARKETING_VERSION` in `project.pbxproj`, `CFBundleShortVersionString` in `Info.plist`

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

[release]
enabled = false
on_bump = []
match = ""
skip = ""
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

## CI / Release workflows

`ssmver` can generate GitHub Actions workflow files that automatically release or publish your project when a version bump is merged to your main branch.

Each command generates a separate workflow in `.github/workflows/`:

```bash
ssmver release    # GitHub Release (git tag + release page) -> release.yaml
ssmver package    # GitHub Packages (npm, Maven, NuGet, Ruby) -> package.yaml
ssmver npm        # Publish to npmjs.com                      -> npm.yaml
ssmver crates     # Publish to crates.io                      -> crates.yaml
```

The ecosystem-specific commands validate that the project actually uses that ecosystem before generating. For example, `ssmver npm` requires a `package.json` in the repo, and `ssmver crates` requires a `Cargo.toml`.

The `ssmver package` command supports ecosystems that have a GitHub Packages registry: Node, Maven, Gradle, .NET, and Ruby. It does not support Cargo, Python, or PHP since GitHub Packages has no registry for those.

### Workflow triggers

Generated workflows trigger on push to the detected main branch (whichever of `main` or `master` exists in the repo). They only proceed when the version in `ssmver.toml` changed compared to the previous commit.

### Release conditions

The `[release]` section in `ssmver.toml` controls when workflows actually release:

```toml
[release]
enabled = true
on_bump = []
match = ""
skip = ""
```

| Setting | Default | Description |
|---------|---------|-------------|
| `enabled` | `false` | Master toggle. Set to `true` automatically on first `ssmver release`/`npm`/`crates`/`package` run. |
| `on_bump` | `[]` | Bump levels that trigger a release. Empty means every bump (patch, minor, major) triggers a release. Set to `["minor", "major"]` to skip patch-only bumps. |
| `match` | `""` | Only release when the commit message contains this string. Example: `"[release]"`. |
| `skip` | `""` | Skip the release when the commit message contains this string. Example: `"[skip-release]"`. |

These conditions compose: all specified conditions must pass for a release to happen.

Change settings with `ssmver config`:

```bash
ssmver config release.on_bump "minor,major"
ssmver config release.match "[release]"
ssmver config release.skip "[skip-release]"
```

After changing release settings, re-run the workflow command (e.g., `ssmver release`) to regenerate the workflow YAML with the updated filters baked in.

### Secrets

| Command | Required secret |
|---------|----------------|
| `ssmver release` | None (uses `GITHUB_TOKEN`) |
| `ssmver package` | None (uses `GITHUB_TOKEN`) |
| `ssmver npm` | `NPM_TOKEN` |
| `ssmver crates` | `CARGO_REGISTRY_TOKEN` |

## Notes

- `ssmver.toml` is meant to be committed so the team shares the same versioning rules.
- `.ssmver/` is generated and should stay ignored.
- Running `ssmver init` again is safe and refreshes the installed hook wrappers.
- If `ssmver` finds a supported file with a dynamic or ambiguous version expression, it will refuse to sync until you exclude or simplify that file.
