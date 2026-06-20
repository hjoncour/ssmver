use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use tempfile::TempDir;

fn ssmver_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ssmver"))
}

fn init_git_repo(path: &Path) {
    assert_success(run(Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(path)));
    assert_success(run(Command::new("git")
        .args(["config", "user.name", "Codex Tester"])
        .current_dir(path)));
    assert_success(run(Command::new("git")
        .args(["config", "user.email", "codex@example.com"])
        .current_dir(path)));
}

fn run_ssmver(repo: &Path, args: &[&str]) -> Output {
    run(Command::new(ssmver_bin()).args(args).current_dir(repo))
}

fn run_git(repo: &Path, args: &[&str]) -> Output {
    run(Command::new("git").args(args).current_dir(repo))
}

fn run(command: &mut Command) -> Output {
    command.output().expect("command should run")
}

fn assert_success(output: Output) -> String {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_failure(output: Output) -> String {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn init_imports_existing_cargo_version() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.4.2\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));

    let config = fs::read_to_string(temp.path().join("ssmver.toml")).unwrap();
    assert!(config.contains("version = \"1.4.2\""));
}

#[test]
fn init_fails_on_conflicting_versions() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("package.json"),
        "{\n  \"name\": \"demo\",\n  \"version\": \"2.0.0\"\n}\n",
    )
    .unwrap();

    let output = assert_failure(run_ssmver(temp.path(), &["init"]));
    assert!(output.contains("Detected conflicting versions"));
}

#[test]
fn bump_updates_node_workspace_and_lockfile() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::create_dir_all(temp.path().join("packages/web")).unwrap();
    fs::write(
        temp.path().join("package.json"),
        "{\n  \"name\": \"root\",\n  \"version\": \"2.0.0\",\n  \"workspaces\": [\"packages/*\"]\n}\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("package-lock.json"),
        "{\n  \"name\": \"root\",\n  \"version\": \"2.0.0\",\n  \"lockfileVersion\": 3,\n  \"packages\": {\n    \"\": {\n      \"name\": \"root\",\n      \"version\": \"2.0.0\"\n    },\n    \"packages/web\": {\n      \"version\": \"2.0.0\"\n    }\n  }\n}\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("packages/web/package.json"),
        "{\n  \"name\": \"web\",\n  \"version\": \"2.0.0\"\n}\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    assert_success(run_ssmver(temp.path(), &["bump", "minor"]));

    assert!(fs::read_to_string(temp.path().join("package.json"))
        .unwrap()
        .contains("\"version\": \"2.1.0\""));
    assert!(
        fs::read_to_string(temp.path().join("packages/web/package.json"))
            .unwrap()
            .contains("\"version\": \"2.1.0\"")
    );
    let lockfile = fs::read_to_string(temp.path().join("package-lock.json")).unwrap();
    assert!(lockfile.contains("\"version\": \"2.1.0\""));
    assert!(lockfile.contains("\"packages/web\": {\n      \"version\": \"2.1.0\""));
}

#[test]
fn commit_hook_amends_managed_files() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    assert_success(run_git(
        temp.path(),
        &[
            "add",
            "Cargo.toml",
            "src/main.rs",
            "ssmver.toml",
            ".gitignore",
        ],
    ));
    assert_success(run_git(temp.path(), &["commit", "-m", "fix: first commit"]));

    let cargo_show = assert_success(run_git(temp.path(), &["show", "HEAD:Cargo.toml"]));
    let config_show = assert_success(run_git(temp.path(), &["show", "HEAD:ssmver.toml"]));
    assert!(cargo_show.contains("version = \"0.1.1\""));
    assert!(config_show.contains("version = \"0.1.1\""));
}

#[test]
fn dynamic_gradle_targets_block_sync() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("build.gradle"),
        "version = findProperty('releaseVersion')\n",
    )
    .unwrap();

    let output = assert_failure(run_ssmver(temp.path(), &["init"]));
    assert!(output.contains("dynamic or unsupported version assignment"));
}

#[test]
fn update_refreshes_hooks_and_resyncs_files() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    fs::write(
        temp.path().join(".ssmver/hooks/prepare-commit-msg"),
        "#!/bin/sh\necho stale\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"9.9.9\"\nedition = \"2021\"\n",
    )
    .unwrap();

    let output = assert_success(run_ssmver(temp.path(), &["update"]));
    assert!(output.contains("Generated hook wrappers"));
    assert!(output.contains("Re-synced"));

    let hook = fs::read_to_string(temp.path().join(".ssmver/hooks/prepare-commit-msg")).unwrap();
    assert!(hook.contains("hook prepare-commit-msg"));
    let cargo_toml = fs::read_to_string(temp.path().join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("version = \"0.1.0\""));
}

#[test]
fn bump_updates_maven_parent_versions() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::create_dir_all(temp.path().join("module-a")).unwrap();
    fs::write(
        temp.path().join("pom.xml"),
        "<project><modelVersion>4.0.0</modelVersion><groupId>demo</groupId><artifactId>root</artifactId><version>1.0.0</version></project>\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("module-a/pom.xml"),
        "<project><modelVersion>4.0.0</modelVersion><parent><groupId>demo</groupId><artifactId>root</artifactId><version>1.0.0</version><relativePath>../pom.xml</relativePath></parent><artifactId>module-a</artifactId></project>\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    assert_success(run_ssmver(temp.path(), &["bump", "patch"]));

    assert!(fs::read_to_string(temp.path().join("pom.xml"))
        .unwrap()
        .contains("<version>1.0.1</version>"));
    assert!(fs::read_to_string(temp.path().join("module-a/pom.xml"))
        .unwrap()
        .contains("<version>1.0.1</version>"));
}

#[test]
fn bump_updates_phase_two_supported_files() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::create_dir_all(temp.path().join("package")).unwrap();
    fs::create_dir_all(temp.path().join("Properties")).unwrap();
    fs::create_dir_all(temp.path().join("lib/demo")).unwrap();

    fs::write(
        temp.path().join("pyproject.toml"),
        "[project]\nname = \"demo\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("package/__init__.py"),
        "__version__ = \"1.2.3\"\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("demo.csproj"),
        "<Project><PropertyGroup><Version>1.2.3</Version></PropertyGroup></Project>\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("Properties/AssemblyInfo.cs"),
        "[assembly: AssemblyVersion(\"1.2.3.0\")]\n[assembly: AssemblyFileVersion(\"1.2.3.0\")]\n[assembly: AssemblyInformationalVersion(\"1.2.3\")]\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("demo.gemspec"),
        "Gem::Specification.new do |spec|\n  spec.version = \"1.2.3\"\nend\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("lib/demo/version.rb"),
        "module Demo\n  VERSION = \"1.2.3\"\nend\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("composer.json"),
        "{\n  \"name\": \"demo/demo\",\n  \"version\": \"1.2.3\"\n}\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    assert_success(run_ssmver(temp.path(), &["bump", "minor"]));

    assert!(fs::read_to_string(temp.path().join("pyproject.toml"))
        .unwrap()
        .contains("version = \"1.3.0\""));
    assert!(fs::read_to_string(temp.path().join("package/__init__.py"))
        .unwrap()
        .contains("__version__ = \"1.3.0\""));
    assert!(fs::read_to_string(temp.path().join("demo.csproj"))
        .unwrap()
        .contains("<Version>1.3.0</Version>"));
    let assembly_info = fs::read_to_string(temp.path().join("Properties/AssemblyInfo.cs")).unwrap();
    assert!(assembly_info.contains("AssemblyVersion(\"1.3.0.0\")"));
    assert!(assembly_info.contains("AssemblyInformationalVersion(\"1.3.0\")"));
    assert!(fs::read_to_string(temp.path().join("demo.gemspec"))
        .unwrap()
        .contains("spec.version = \"1.3.0\""));
    assert!(fs::read_to_string(temp.path().join("lib/demo/version.rb"))
        .unwrap()
        .contains("VERSION = \"1.3.0\""));
    assert!(fs::read_to_string(temp.path().join("composer.json"))
        .unwrap()
        .contains("\"version\": \"1.3.0\""));
}

#[test]
fn release_generates_workflow_for_any_project() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_success(run_ssmver(temp.path(), &["release"]));
    assert!(output.contains("Generated .github/workflows/release.yaml"));

    let workflow = fs::read_to_string(temp.path().join(".github/workflows/release.yaml")).unwrap();
    assert!(workflow.contains("softprops/action-gh-release@v2"));
    assert!(workflow.contains("Evaluate release conditions"));

    let config = fs::read_to_string(temp.path().join("ssmver.toml")).unwrap();
    assert!(config.contains("enabled = true"));
}

#[test]
fn crates_generates_workflow_with_cargo_project() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_success(run_ssmver(temp.path(), &["crates"]));
    assert!(output.contains("Generated .github/workflows/crates.yaml"));
    assert!(output.contains("CARGO_REGISTRY_TOKEN"));

    let workflow = fs::read_to_string(temp.path().join(".github/workflows/crates.yaml")).unwrap();
    assert!(workflow.contains("cargo publish"));
}

#[test]
fn crates_fails_without_cargo_project() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("package.json"),
        "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\"\n}\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_failure(run_ssmver(temp.path(), &["crates"]));
    assert!(output.contains("No Cargo targets found"));
}

#[test]
fn npm_generates_workflow_with_node_project() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("package.json"),
        "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\"\n}\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_success(run_ssmver(temp.path(), &["npm"]));
    assert!(output.contains("Generated .github/workflows/npm.yaml"));
    assert!(output.contains("NPM_TOKEN"));

    let workflow = fs::read_to_string(temp.path().join(".github/workflows/npm.yaml")).unwrap();
    assert!(workflow.contains("npm publish"));
}

#[test]
fn npm_fails_without_node_project() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_failure(run_ssmver(temp.path(), &["npm"]));
    assert!(output.contains("No Node targets found"));
}

#[test]
fn package_fails_without_supported_ecosystem() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_failure(run_ssmver(temp.path(), &["package"]));
    assert!(output.contains("No ecosystem supporting GitHub Packages found"));
}

#[test]
fn package_generates_workflow_for_node_project() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("package.json"),
        "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\"\n}\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_success(run_ssmver(temp.path(), &["package"]));
    assert!(output.contains("Generated .github/workflows/package.yaml"));

    let workflow = fs::read_to_string(temp.path().join(".github/workflows/package.yaml")).unwrap();
    assert!(workflow.contains("npm.pkg.github.com"));
    assert!(workflow.contains("packages: write"));
}

#[test]
fn release_overwrites_existing_workflow() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    assert_success(run_ssmver(temp.path(), &["release"]));

    let output = assert_success(run_ssmver(temp.path(), &["release"]));
    assert!(output.contains("Overwrote .github/workflows/release.yaml"));
}

#[test]
fn bump_updates_xcode_marketing_version() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::create_dir_all(temp.path().join("App.xcodeproj")).unwrap();
    fs::write(
        temp.path().join("App.xcodeproj/project.pbxproj"),
        "// !$*UTF8*$!\n{\n\tobjectVersion = 56;\n\tobjects = {\n\t\tBuildConfig1 = {\n\t\t\tbuildSettings = {\n\t\t\t\tMARKETING_VERSION = 2.0.0;\n\t\t\t};\n\t\t};\n\t\tBuildConfig2 = {\n\t\t\tbuildSettings = {\n\t\t\t\tMARKETING_VERSION = 2.0.0;\n\t\t\t};\n\t\t};\n\t};\n}\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let config = fs::read_to_string(temp.path().join("ssmver.toml")).unwrap();
    assert!(config.contains("version = \"2.0.0\""));

    assert_success(run_ssmver(temp.path(), &["bump", "minor"]));

    let pbxproj = fs::read_to_string(temp.path().join("App.xcodeproj/project.pbxproj")).unwrap();
    assert!(pbxproj.contains("MARKETING_VERSION = 2.1.0;"));
    assert!(!pbxproj.contains("MARKETING_VERSION = 2.0.0;"));
}

#[test]
fn bump_updates_info_plist_literal_version() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Info.plist"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>CFBundleShortVersionString</key>\n\t<string>1.0.0</string>\n</dict>\n</plist>\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    assert_success(run_ssmver(temp.path(), &["bump", "major"]));

    let plist = fs::read_to_string(temp.path().join("Info.plist")).unwrap();
    assert!(plist.contains("<string>2.0.0</string>"));
}

#[test]
fn info_plist_with_build_variable_is_skipped() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::create_dir_all(temp.path().join("App.xcodeproj")).unwrap();
    fs::write(
        temp.path().join("App.xcodeproj/project.pbxproj"),
        "// pbxproj\n{\n\tBuildConfig = {\n\t\tbuildSettings = {\n\t\t\tMARKETING_VERSION = 1.0.0;\n\t\t};\n\t};\n}\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("Info.plist"),
        "<?xml version=\"1.0\"?>\n<plist version=\"1.0\">\n<dict>\n\t<key>CFBundleShortVersionString</key>\n\t<string>$(MARKETING_VERSION)</string>\n</dict>\n</plist>\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    assert_success(run_ssmver(temp.path(), &["bump", "patch"]));

    let pbxproj = fs::read_to_string(temp.path().join("App.xcodeproj/project.pbxproj")).unwrap();
    assert!(pbxproj.contains("MARKETING_VERSION = 1.0.1;"));

    let plist = fs::read_to_string(temp.path().join("Info.plist")).unwrap();
    assert!(plist.contains("$(MARKETING_VERSION)"));
}

#[test]
fn status_reports_all_in_sync() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_success(run_ssmver(temp.path(), &["status"]));
    assert!(output.contains("version: 1.0.0"));
    assert!(output.contains("1 target(s) in sync"));
}

#[test]
fn status_detects_drift() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));

    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"9.9.9\"\nedition = \"2021\"\n",
    )
    .unwrap();

    let output = run_ssmver(temp.path(), &["status"]);
    assert!(!output.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("Out of sync"));
    assert!(text.contains("9.9.9"));
    assert!(text.contains("expected 1.0.0"));
}

#[test]
fn bare_ssmver_runs_status_when_initialized() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_success(run_ssmver(temp.path(), &[]));
    assert!(output.contains("version: 1.0.0"));
    assert!(output.contains("in sync"));
}

#[test]
fn init_creates_config_with_changelog_disabled() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));

    let config = fs::read_to_string(temp.path().join("ssmver.toml")).unwrap();
    assert!(config.contains("[changelog]"));
    assert!(config.contains("enabled = false"));
    assert!(config.contains("editor = \"editor\""));
    assert!(config.contains("# Enable changelog entry collection on version bumps"));
    assert!(config.contains("# How to collect changelog entries"));
}

#[test]
fn changelog_enables_section_in_existing_project() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));

    let config_before = fs::read_to_string(temp.path().join("ssmver.toml")).unwrap();
    assert!(config_before.contains("enabled = false"));

    let output = assert_success(run_ssmver(temp.path(), &["changelog"]));
    assert!(output.contains("Enabled [changelog] in ssmver.toml"));

    let config_after = fs::read_to_string(temp.path().join("ssmver.toml")).unwrap();
    assert!(config_after.contains("[changelog]"));
    let changelog_section = config_after.split("[changelog]").nth(1).unwrap();
    assert!(changelog_section.contains("enabled = true"));
    assert!(config_after.contains("# Enable changelog entry collection on version bumps"));
    assert!(config_after.contains("# How to collect changelog entries"));
}

#[test]
fn changelog_is_idempotent() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    assert_success(run_ssmver(temp.path(), &["changelog"]));

    let output = assert_success(run_ssmver(temp.path(), &["changelog"]));
    assert!(output.contains("Changelog is already enabled"));
}

#[test]
fn marketplace_generates_workflow_for_vscode_extension() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("package.json"),
        r#"{
  "name": "my-ext",
  "version": "0.1.0",
  "publisher": "test-publisher",
  "engines": { "vscode": "^1.80.0" }
}"#,
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));

    let targets_output = assert_success(run_ssmver(temp.path(), &["targets", "list", "--json"]));
    assert!(targets_output.contains("\"vscode\""));

    let output = assert_success(run_ssmver(temp.path(), &["marketplace"]));
    assert!(output.contains("Generated .github/workflows/marketplace.yaml"));
    assert!(output.contains("VSCE_PAT"));
    assert!(output.contains("OVSX_TOKEN"));

    let workflow =
        fs::read_to_string(temp.path().join(".github/workflows/marketplace.yaml")).unwrap();
    assert!(workflow.contains("@vscode/vsce package"));
    assert!(workflow.contains("@vscode/vsce publish"));
    assert!(workflow.contains("ovsx publish"));
}

#[test]
fn marketplace_fails_without_vscode_extension() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("package.json"),
        "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\"\n}\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_failure(run_ssmver(temp.path(), &["marketplace"]));
    assert!(output.contains("No VSCode targets found"));
}

#[test]
fn vscode_extension_detected_as_vscode_ecosystem() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("package.json"),
        r#"{
  "name": "my-ext",
  "version": "1.2.3",
  "publisher": "test-publisher",
  "engines": { "vscode": "^1.80.0" }
}"#,
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_success(run_ssmver(temp.path(), &["targets", "list", "--json"]));
    assert!(output.contains("\"vscode\""));
    assert!(!output.contains("\"node\""));
}

#[test]
fn plain_node_project_not_detected_as_vscode() {
    let temp = TempDir::new().unwrap();
    init_git_repo(temp.path());
    fs::write(
        temp.path().join("package.json"),
        "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\"\n}\n",
    )
    .unwrap();

    assert_success(run_ssmver(temp.path(), &["init"]));
    let output = assert_success(run_ssmver(temp.path(), &["targets", "list", "--json"]));
    assert!(output.contains("\"node\""));
    assert!(!output.contains("\"vscode\""));
}
