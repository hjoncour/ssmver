# ssmver

`ssmver` is a Rust CLI that installs automatic semantic versioning into any git repository.

Once you run `ssmver init` inside a project, it creates a shared `ssmver.toml` config file, installs local git hooks, and updates the repo so commits with recognized prefixes automatically bump the project version.

Examples:

- `fix: correct null handling` -> patch bump
- `feature(auth): add OAuth login` -> minor bump
- `release(v2): break old API` -> major bump

The generated hooks are shell scripts stored in `.ssmver/`, so collaborators do not need to invoke the `ssmver` binary on every commit after setup.

## How it works

Running `ssmver init` in a git repository will:

1. Create `.ssmver/hooks/` and `.ssmver/scripts/`
2. Generate the git hook scripts used for version bumping
3. Set `git config core.hooksPath .ssmver/hooks`
4. Add `.ssmver/` to `.gitignore`
5. Create `ssmver.toml` if it does not already exist

The committed `ssmver.toml` file holds the current version and prefix rules. By default it starts with:

```toml
version = "0.1.0"

[settings]
mode = "branch"
prompt = "never"
prompt_prefixes = ["feature", "release"]

[prefixes]
fix = "patch"
feature = "minor"
release = "major"
```

## Installation

`ssmver` currently installs through Cargo.

### Install from a local checkout

```bash
git clone https://github.com/hjoncour/ssmver.git
cd ssmver
cargo install --path .
```

### Install directly from GitHub

```bash
cargo install --git https://github.com/hjoncour/ssmver.git
```

### Development usage without installing

```bash
cargo run -- init
```

## Requirements

- Rust and Cargo
- Git
- A repository where you want automatic version bumps

## Quick start

Install `ssmver`, then move into the repository you want to configure:

```bash
cd /path/to/your/project
ssmver init
```

After that, normal commits can drive version changes automatically:

```bash
git commit -m "fix: handle empty response"
git commit -m "feature(api): add search endpoint"
git commit -m "release(v2): remove deprecated routes"
```

You can also manage the config manually:

```bash
ssmver prefix add hotfix patch
ssmver prefix remove hotfix
ssmver prefix list
ssmver bump minor
ssmver version
ssmver config mode all
ssmver uninstall
```

## Commands

```text
ssmver init
ssmver prefix add <name> <patch|minor|major>
ssmver prefix remove <name>
ssmver prefix list
ssmver bump <patch|minor|major>
ssmver version
ssmver config <key> [value]
ssmver uninstall
```

## Notes

- `ssmver.toml` is meant to be committed so the team shares the same rules.
- `.ssmver/` is generated and should stay ignored.
- Running `ssmver init` again is safe and regenerates the local hook scripts.
