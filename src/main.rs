use clap::{CommandFactory, Parser, Subcommand};
use semver::{Version, VersionReq};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use walkdir::{DirEntry, WalkDir};

#[derive(Parser, Debug)]
#[command(
    name = "loadout",
    version,
    about = "Check a repository's local development requirements"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Inspect repository metadata and local prerequisites
    Check {
        /// Repository directory (defaults to the current directory)
        path: Option<PathBuf>,
        /// Add a one-off requirement: cmd:NAME, cmd:NAME@VERSION, or env:NAME
        #[arg(long = "require", value_name = "REQUIREMENT")]
        requirements: Vec<String>,
        /// Emit a machine-readable report
        #[arg(long)]
        json: bool,
        /// Disable ANSI colors in human output
        #[arg(long)]
        no_color: bool,
    },
    /// Print an advisory summary of detected requirements without creating files
    Init {
        /// Repository directory (defaults to the current directory)
        path: Option<PathBuf>,
        /// Emit a machine-readable advisory
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Command,
    Environment,
    DependencyState,
}

#[derive(Clone, Debug)]
struct Requirement {
    kind: Kind,
    name: String,
    constraint: Option<String>,
    source: String,
    required: bool,
    message: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Pass,
    Fail,
    Warn,
}

#[derive(Debug, Serialize)]
struct ResultItem {
    status: Status,
    kind: Kind,
    name: String,
    constraint: Option<String>,
    source: String,
    found: Option<String>,
    message: String,
}

#[derive(Serialize)]
struct Report {
    path: String,
    results: Vec<ResultItem>,
    passed: usize,
    failed: usize,
    warnings: usize,
}

#[derive(Serialize)]
struct Advisory {
    path: String,
    writes_files: bool,
    requirements: Vec<AdvisoryItem>,
}

#[derive(Serialize)]
struct AdvisoryItem {
    requirement: String,
    source: String,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Check {
            path,
            requirements,
            json,
            no_color,
        }) => run_check(resolve_root(path), requirements, json, no_color),
        Some(Commands::Init { path, json }) => run_init(resolve_root(path), json),
        None => {
            let mut cmd = Cli::command();
            cmd.print_help().expect("stdout is available");
            println!();
        }
    }
}

fn resolve_root(path: Option<PathBuf>) -> PathBuf {
    let path = path.unwrap_or_else(|| env::current_dir().expect("current directory is available"));
    match path.canonicalize() {
        Ok(path) if path.is_dir() => path,
        _ => {
            eprintln!("loadout: '{}' is not a readable directory", path.display());
            std::process::exit(2);
        }
    }
}

fn run_check(root: PathBuf, requirements: Vec<String>, json: bool, no_color: bool) {
    let mut diagnostics = Vec::new();
    let mut requirements_found = discover(&root, &mut diagnostics);
    for input in requirements {
        match parse_custom_requirement(&input) {
            Ok(requirement) => requirements_found.push(requirement),
            Err(message) => {
                eprintln!("loadout: {message}");
                std::process::exit(2);
            }
        }
    }
    let env_values = read_local_env(&root);
    let mut results: Vec<_> = requirements_found
        .iter()
        .map(|r| evaluate(r, &root, &env_values))
        .collect();
    results.append(&mut diagnostics);
    results.sort_by(|a, b| a.name.cmp(&b.name).then(a.source.cmp(&b.source)));
    let report = Report {
        path: root.display().to_string(),
        passed: results
            .iter()
            .filter(|r| matches!(r.status, Status::Pass))
            .count(),
        failed: results
            .iter()
            .filter(|r| matches!(r.status, Status::Fail))
            .count(),
        warnings: results
            .iter()
            .filter(|r| matches!(r.status, Status::Warn))
            .count(),
        results,
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("report serializes")
        );
    } else {
        print_human(&report, !no_color);
    }
    if report.failed > 0 {
        std::process::exit(1);
    }
}

fn run_init(root: PathBuf, json: bool) {
    let mut diagnostics = Vec::new();
    let mut requirements = discover(&root, &mut diagnostics);
    requirements.sort_by(|a, b| a.name.cmp(&b.name).then(a.source.cmp(&b.source)));
    requirements.dedup_by(|a, b| {
        a.kind == b.kind && a.name == b.name && a.constraint == b.constraint && a.source == b.source
    });
    let advisory = Advisory {
        path: display(&root),
        writes_files: false,
        requirements: requirements.iter().filter_map(advisory_item).collect(),
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&advisory).expect("advisory serializes")
        );
        return;
    }
    println!("Loadout advisory for {}", advisory.path);
    println!("No files were created. Loadout reads these existing project signals automatically:");
    if advisory.requirements.is_empty() {
        println!("  No supported requirements were detected.");
    } else {
        for item in advisory.requirements {
            println!("  - {} ({})", item.requirement, item.source);
        }
    }
    if !diagnostics.is_empty() {
        println!("\nSome metadata could not be parsed; run `loadout check` for details.");
    }
    println!(
        "\nRun `loadout check` to validate this environment. Use `--require` only for one-off requirements."
    );
}

fn advisory_item(requirement: &Requirement) -> Option<AdvisoryItem> {
    let rendered = match requirement.kind {
        Kind::Command => match &requirement.constraint {
            Some(constraint) => format!("cmd:{}@{constraint}", requirement.name),
            None => format!("cmd:{}", requirement.name),
        },
        Kind::Environment => format!("env:{}", requirement.name),
        Kind::DependencyState => return None,
    };
    Some(AdvisoryItem {
        requirement: rendered,
        source: requirement.source.clone(),
    })
}

fn ignored(entry: &DirEntry) -> bool {
    entry.file_type().is_dir()
        && matches!(
            entry.file_name().to_string_lossy().as_ref(),
            ".git"
                | "node_modules"
                | "target"
                | ".venv"
                | "venv"
                | "__pycache__"
                | "cdk.out"
                | "dist"
                | "build"
                | "generated"
        )
}

fn discover(root: &Path, diagnostics: &mut Vec<ResultItem>) -> Vec<Requirement> {
    let mut found = Vec::new();
    let mut node_projects = Vec::new();
    let mut rust_projects = Vec::new();
    let mut python_projects = Vec::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !ignored(e))
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy();
        match name.as_ref() {
            "package.json" => {
                node_projects.push(path.parent().unwrap().to_path_buf());
                found.extend(node_requirements(path, diagnostics));
            }
            ".nvmrc" | ".node-version" => {
                if let Ok(value) = fs::read_to_string(path) {
                    let version = value.trim().trim_start_matches('v');
                    if !version.is_empty() {
                        found.push(command(
                            "node",
                            Some(node_version_constraint(version)),
                            display(path),
                            true,
                        ));
                    }
                }
            }
            "rust-toolchain" | "rust-toolchain.toml" => {
                rust_projects.push(path.parent().unwrap().to_path_buf());
                found.extend(rust_toolchain_requirements(path));
            }
            "Cargo.toml" => {
                rust_projects.push(path.parent().unwrap().to_path_buf());
                found.extend(cargo_requirements(path));
            }
            ".python-version" => {
                if let Ok(value) = fs::read_to_string(path) {
                    let version = value.trim();
                    if !version.is_empty() {
                        found.push(command(
                            "python",
                            Some(normalize_exact(version)),
                            display(path),
                            true,
                        ));
                    }
                }
            }
            "pyproject.toml" => {
                python_projects.push(path.parent().unwrap().to_path_buf());
                found.extend(pyproject_requirements(path));
            }
            "go.mod" => {
                found.push(command("go", go_mod_constraint(path), display(path), true));
            }
            "pom.xml" | "build.gradle" | "build.gradle.kts" | "gradlew" => {
                found.push(command("java", None, display(path), true));
            }
            ".ruby-version" => {
                if let Ok(version) = fs::read_to_string(path) {
                    let version = version.trim();
                    if !version.is_empty() {
                        found.push(command(
                            "ruby",
                            Some(normalize_exact(version)),
                            display(path),
                            true,
                        ));
                    }
                }
            }
            "Gemfile" => found.push(command("ruby", None, display(path), true)),
            "Gemfile.lock" => found.push(command("bundle", None, display(path), true)),
            "Dockerfile"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "compose.yml"
            | "compose.yaml" => {
                found.push(command("docker", None, display(path), true));
            }
            _ if name.starts_with("Dockerfile.") => {
                found.push(command("docker", None, display(path), true))
            }
            _ if name.ends_with(".tf") => {
                found.push(command("terraform", None, display(path), true))
            }
            ".psqlrc" | "postgresql.conf" => found.push(command("psql", None, display(path), true)),
            ".rediscli_history" | "redis.conf" => {
                found.push(command("redis-cli", None, display(path), true))
            }
            "uv.lock" => found.push(command("uv", None, display(path), true)),
            "poetry.lock" => found.push(command("poetry", None, display(path), true)),
            "Pipfile" | "Pipfile.lock" => found.push(command("pipenv", None, display(path), true)),
            "package-lock.json" => found.push(command("npm", None, display(path), true)),
            "pnpm-lock.yaml" => found.push(command("pnpm", None, display(path), true)),
            "yarn.lock" => found.push(command("yarn", None, display(path), true)),
            "bun.lockb" | "bun.lock" => found.push(command("bun", None, display(path), true)),
            _ => {
                if name.starts_with("requirements")
                    && (name.ends_with(".txt") || name == "requirements")
                {
                    python_projects.push(path.parent().unwrap().to_path_buf());
                    found.push(command("python", None, display(path), true));
                }
            }
        }
    }
    for path in node_projects {
        dependency_warning(
            &mut found,
            &path,
            "node_modules",
            ".pnp.cjs",
            "Node dependencies do not appear to be installed",
        );
    }
    for path in rust_projects {
        dependency_warning(
            &mut found,
            &path,
            "target",
            "",
            "Cargo build artifacts are absent; run cargo build or cargo test",
        );
    }
    for path in python_projects {
        dependency_warning(
            &mut found,
            &path,
            ".venv",
            "venv",
            "A local Python virtual environment is absent",
        );
    }
    found.extend(env_requirements(root));
    found
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
fn command(name: &str, constraint: Option<String>, source: String, required: bool) -> Requirement {
    Requirement {
        kind: Kind::Command,
        name: name.into(),
        constraint,
        source,
        required,
        message: None,
    }
}
fn dependency_warning(
    found: &mut Vec<Requirement>,
    project: &Path,
    primary: &str,
    alternative: &str,
    message: &str,
) {
    if !project.join(primary).exists()
        && (alternative.is_empty() || !project.join(alternative).exists())
    {
        found.push(Requirement {
            kind: Kind::DependencyState,
            name: primary.into(),
            constraint: None,
            source: display(project),
            required: false,
            message: Some(message.into()),
        });
    }
}

fn node_requirements(path: &Path, diagnostics: &mut Vec<ResultItem>) -> Vec<Requirement> {
    let mut found = Vec::new();
    let source = display(path);
    let Ok(value) = fs::read_to_string(path)
        .and_then(|s| serde_json::from_str::<Value>(&s).map_err(std::io::Error::other))
    else {
        diagnostics.push(warn("package.json", source, "Could not parse package.json"));
        return found;
    };
    if let Some(version) = value.pointer("/engines/node").and_then(Value::as_str) {
        found.push(command("node", Some(version.into()), display(path), true));
    }
    if let Some(package_manager) = value.get("packageManager").and_then(Value::as_str) {
        let (name, version) = package_manager
            .rsplit_once('@')
            .unwrap_or((package_manager, ""));
        if !name.is_empty() {
            found.push(command(
                name,
                (!version.is_empty()).then(|| normalize_exact(version)),
                display(path),
                true,
            ));
        }
    }
    for name in ["node", "npm", "pnpm", "yarn"] {
        if let Some(version) = value
            .pointer(&format!("/volta/{name}"))
            .and_then(Value::as_str)
        {
            found.push(command(
                name,
                Some(normalize_exact(version)),
                display(path),
                true,
            ));
        }
    }
    found
}

fn rust_toolchain_requirements(path: &Path) -> Vec<Requirement> {
    let value = fs::read_to_string(path).unwrap_or_default();
    let channel = if path.file_name().unwrap() == "rust-toolchain.toml" {
        toml::from_str::<toml::Value>(&value).ok().and_then(|v| {
            v.get("toolchain")?
                .get("channel")?
                .as_str()
                .map(str::to_owned)
        })
    } else {
        Some(value.trim().to_owned())
    };
    match channel.filter(|v| !v.is_empty() && v != "stable" && v != "beta" && v != "nightly") {
        Some(version) => vec![
            command(
                "rustc",
                Some(normalize_exact(&version)),
                display(path),
                true,
            ),
            command(
                "cargo",
                Some(normalize_exact(&version)),
                display(path),
                true,
            ),
        ],
        None => vec![
            command("rustc", None, display(path), true),
            command("cargo", None, display(path), true),
        ],
    }
}

fn cargo_requirements(path: &Path) -> Vec<Requirement> {
    let constraint = fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
        .and_then(|v| {
            v.get("package")?
                .get("rust-version")?
                .as_str()
                .map(normalize_exact)
        });
    vec![
        command("rustc", constraint, display(path), true),
        command("cargo", None, display(path), true),
    ]
}

fn go_mod_constraint(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().and_then(|contents| {
        contents.lines().find_map(|line| {
            let version = line.trim().strip_prefix("go ")?.trim();
            (!version.is_empty()).then(|| format!(">={}", normalize_version(version)))
        })
    })
}

fn pyproject_requirements(path: &Path) -> Vec<Requirement> {
    let constraint = fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
        .and_then(|v| {
            v.get("project")?
                .get("requires-python")?
                .as_str()
                .map(str::to_owned)
        });
    vec![command("python", constraint, display(path), true)]
}

fn env_requirements(root: &Path) -> Vec<Requirement> {
    [".env.example", ".env.sample"]
        .iter()
        .flat_map(|file| {
            let path = root.join(file);
            let source = display(&path);
            fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .filter_map(move |line| {
                    let line = line.trim().trim_start_matches("export ").trim();
                    let name = line.split_once('=')?.0.trim();
                    (!name.is_empty() && !name.starts_with('#')).then(|| Requirement {
                        kind: Kind::Environment,
                        name: name.into(),
                        constraint: None,
                        source: source.clone(),
                        required: true,
                        message: None,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn read_local_env(root: &Path) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for file in [".env", ".env.local"] {
        if let Ok(contents) = fs::read_to_string(root.join(file)) {
            for line in contents.lines() {
                let line = line.trim().trim_start_matches("export ").trim();
                if let Some((key, value)) = line.split_once('=') {
                    values.insert(
                        key.trim().into(),
                        value.trim().trim_matches(['\'', '"']).into(),
                    );
                }
            }
        }
    }
    values
}

fn parse_custom_requirement(input: &str) -> Result<Requirement, String> {
    if let Some(name) = input.strip_prefix("env:")
        && !name.is_empty()
    {
        return Ok(Requirement {
            kind: Kind::Environment,
            name: name.into(),
            constraint: None,
            source: "--require".into(),
            required: true,
            message: None,
        });
    }
    if let Some(command_spec) = input.strip_prefix("cmd:") {
        let (name, constraint) = command_spec
            .rsplit_once('@')
            .map(|(n, c)| (n, Some(c.to_owned())))
            .unwrap_or((command_spec, None));
        if !name.is_empty() && constraint.as_deref().is_none_or(|c| !c.is_empty()) {
            return Ok(command(name, constraint, "--require".into(), true));
        }
    }
    Err(format!(
        "invalid requirement '{input}'; use cmd:NAME, cmd:NAME@VERSION, or env:NAME"
    ))
}

fn evaluate(
    requirement: &Requirement,
    _root: &Path,
    env_values: &HashMap<String, String>,
) -> ResultItem {
    match requirement.kind {
        Kind::DependencyState => ResultItem {
            status: Status::Warn,
            kind: Kind::DependencyState,
            name: requirement.name.clone(),
            constraint: None,
            source: requirement.source.clone(),
            found: None,
            message: requirement.message.clone().unwrap(),
        },
        Kind::Environment => {
            let value = env::var(&requirement.name)
                .ok()
                .or_else(|| env_values.get(&requirement.name).cloned());
            let present = value.is_some_and(|v| !v.is_empty());
            ResultItem {
                status: if present { Status::Pass } else { Status::Fail },
                kind: Kind::Environment,
                name: requirement.name.clone(),
                constraint: None,
                source: requirement.source.clone(),
                found: present.then(|| "set".into()),
                message: if present {
                    "Environment variable is set".into()
                } else {
                    "Set this environment variable in your shell or local .env file".into()
                },
            }
        }
        Kind::Command => evaluate_command(requirement),
    }
}

fn evaluate_command(requirement: &Requirement) -> ResultItem {
    let commands: Vec<&str> = if requirement.name == "python" {
        vec!["python", "python3"]
    } else {
        vec![&requirement.name]
    };
    let output = commands.iter().find_map(|name| {
        Command::new(name)
            .arg("--version")
            .output()
            .ok()
            .map(|output| ((*name).to_owned(), output))
    });
    let unavailable = if requirement.required {
        Status::Fail
    } else {
        Status::Warn
    };
    let (status, found, message) = match output {
        None => (
            unavailable,
            None,
            format!(
                "Install '{}' and ensure it is available on PATH",
                requirement.name
            ),
        ),
        Some((_, output)) if !output.status.success() => (
            unavailable,
            None,
            format!("'{} --version' did not run successfully", requirement.name),
        ),
        Some((command_name, output)) => {
            let text = String::from_utf8_lossy(&output.stdout).to_string()
                + &String::from_utf8_lossy(&output.stderr);
            let version = extract_version(&text);
            match (&requirement.constraint, version.as_ref()) {
                (Some(constraint), Some(found)) if version_matches(found, constraint) => (
                    Status::Pass,
                    Some(found.clone()),
                    "Version satisfies requirement".into(),
                ),
                (Some(constraint), Some(found)) => (
                    Status::Fail,
                    Some(found.clone()),
                    format!("Installed version does not satisfy '{constraint}'"),
                ),
                (Some(_), None) => (
                    Status::Fail,
                    None,
                    "Could not determine installed version".into(),
                ),
                (None, Some(found)) => (
                    Status::Pass,
                    Some(found.clone()),
                    if command_name == requirement.name {
                        "Executable is available".into()
                    } else {
                        format!("Executable is available as '{command_name}'")
                    },
                ),
                (None, None) => (
                    Status::Pass,
                    None,
                    "Executable is available (version could not be read)".into(),
                ),
            }
        }
    };
    ResultItem {
        status,
        kind: Kind::Command,
        name: requirement.name.clone(),
        constraint: requirement.constraint.clone(),
        source: requirement.source.clone(),
        found,
        message,
    }
}

fn extract_version(text: &str) -> Option<String> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_owned)
}
fn normalize_exact(input: &str) -> String {
    format!(
        "={}",
        normalize_version(input.trim().trim_start_matches('v'))
    )
}

fn node_version_constraint(input: &str) -> String {
    let version = input.trim().trim_start_matches('v');
    let numeric = version
        .split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    if numeric {
        let pieces: Vec<_> = version.split('.').collect();
        if pieces.len() < 3 {
            let lower = normalize_version(version);
            let mut upper: Vec<u64> = pieces.iter().filter_map(|part| part.parse().ok()).collect();
            let last = upper.len() - 1;
            upper[last] += 1;
            let upper = upper
                .into_iter()
                .map(|part| part.to_string())
                .collect::<Vec<_>>()
                .join(".");
            return format!(">={lower}, <{}", normalize_version(&upper));
        }
    }
    normalize_exact(version)
}

fn normalize_version(input: &str) -> String {
    let mut pieces: Vec<_> = input.split('.').collect();
    if pieces
        .first()
        .is_some_and(|p| p.chars().all(|c| c.is_ascii_digit()))
    {
        while pieces.len() < 3 {
            pieces.push("0");
        }
        pieces.join(".")
    } else {
        input.into()
    }
}
fn version_matches(found: &str, constraint: &str) -> bool {
    let found = Version::parse(&normalize_version(found)).ok();
    let normalized = constraint
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (_, version) = part
                .trim_start_matches(['>', '<', '=', '^', '~'])
                .split_at(0);
            let prefix_len = part.len() - version.len();
            let prefix = &part[..prefix_len];
            format!("{prefix}{}", normalize_version(version))
        })
        .collect::<Vec<_>>()
        .join(", ");
    found
        .zip(VersionReq::parse(&normalized).ok())
        .is_some_and(|(version, req)| req.matches(&version))
}

fn warn(name: &str, source: String, message: &str) -> ResultItem {
    ResultItem {
        status: Status::Warn,
        kind: Kind::Command,
        name: name.into(),
        constraint: None,
        source,
        found: None,
        message: message.into(),
    }
}
fn print_human(report: &Report, color: bool) {
    println!("Loadout: {}", report.path);
    for item in &report.results {
        let (label, code) = match item.status {
            Status::Pass => ("PASS", "32"),
            Status::Fail => ("FAIL", "31"),
            Status::Warn => ("WARN", "33"),
        };
        let label = if color {
            format!("\x1b[{code}m{label}\x1b[0m")
        } else {
            label.into()
        };
        let constraint = item
            .constraint
            .as_ref()
            .map(|c| format!(" ({c})"))
            .unwrap_or_default();
        let found = item
            .found
            .as_ref()
            .map(|v| format!(" [{v}]"))
            .unwrap_or_default();
        println!(
            "{label:>13}  {}{constraint}{found}\n                {}  ({})",
            item.name, item.message, item.source
        );
    }
    println!(
        "\n{} passed, {} failed, {} warnings",
        report.passed, report.failed, report.warnings
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_requirements_parse() {
        assert_eq!(
            parse_custom_requirement("cmd:node@>=22")
                .unwrap()
                .constraint
                .as_deref(),
            Some(">=22")
        );
        assert_eq!(
            parse_custom_requirement("env:DATABASE_URL").unwrap().kind,
            Kind::Environment
        );
        assert!(parse_custom_requirement("node@22").is_err());
    }
    #[test]
    fn version_constraints_match() {
        assert!(version_matches("22.4.1", ">=22"));
        assert!(version_matches("3.12.2", ">=3.10"));
        assert!(!version_matches("20.0.0", ">=22"));
        assert!(version_matches("1.94.0", "=1.94"));
        assert!(version_matches("22.20.0", &node_version_constraint("22")));
        assert!(!version_matches("23.0.0", &node_version_constraint("22")));
    }
    #[test]
    fn version_extraction_is_safe() {
        assert_eq!(extract_version("node v22.3.1"), Some("22.3.1".into()));
        assert_eq!(extract_version("stable"), None);
    }
    #[test]
    fn advisory_renders_only_actionable_requirements() {
        let command = command("node", Some(">=22".into()), "package.json".into(), true);
        assert_eq!(
            advisory_item(&command).unwrap().requirement,
            "cmd:node@>=22"
        );
        let state = Requirement {
            kind: Kind::DependencyState,
            name: "node_modules".into(),
            constraint: None,
            source: "project".into(),
            required: false,
            message: None,
        };
        assert!(advisory_item(&state).is_none());
    }
    #[test]
    fn reads_go_version_from_module_file() {
        let path = std::env::temp_dir().join("loadout-go-mod-test");
        fs::write(&path, "module example.com/demo\n\ngo 1.23\n").unwrap();
        assert_eq!(go_mod_constraint(&path).as_deref(), Some(">=1.23.0"));
        fs::remove_file(path).unwrap();
    }
}
