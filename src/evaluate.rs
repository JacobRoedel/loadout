use std::{collections::HashMap, env, path::Path, process::Command};

use semver::{Version, VersionReq};

use crate::connectivity::evaluate_connectivity;
use crate::model::{Kind, Requirement, ResultItem, Status, command};

pub(crate) fn parse_custom_requirement(input: &str) -> Result<Requirement, String> {
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

pub(crate) fn evaluate(
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
            let missing_status = if requirement.required {
                Status::Fail
            } else {
                Status::Warn
            };
            ResultItem {
                status: if present {
                    Status::Pass
                } else {
                    missing_status
                },
                kind: Kind::Environment,
                name: requirement.name.clone(),
                constraint: None,
                source: requirement.source.clone(),
                found: present.then(|| "set".into()),
                message: if present {
                    "Environment variable is set".into()
                } else if requirement.required {
                    "Set this environment variable in your shell or local .env file".into()
                } else {
                    "Optional environment variable is not set".into()
                },
            }
        }
        Kind::Command => evaluate_command(requirement),
        Kind::Connectivity => evaluate_connectivity(requirement),
    }
}

pub(crate) fn evaluate_command(requirement: &Requirement) -> ResultItem {
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
            missing_command_message(&requirement.name),
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

pub(crate) fn missing_command_message(name: &str) -> String {
    let generic = format!("Install '{name}' and ensure it is available on PATH");
    installation_hint_for(env::consts::OS, name)
        .map(|hint| format!("{generic}; try `{hint}`"))
        .unwrap_or(generic)
}

pub(crate) fn installation_hint_for(os: &str, name: &str) -> Option<&'static str> {
    match (os, name) {
        ("macos", "node") => Some("brew install node"),
        ("macos", "npm") => Some("brew install node"),
        ("macos", "pnpm") => Some("brew install pnpm"),
        ("macos", "yarn") => Some("brew install yarn"),
        ("macos", "bun") => Some("brew install oven-sh/bun/bun"),
        ("macos", "rustc" | "cargo") => Some("brew install rust"),
        ("macos", "python") => Some("brew install python"),
        ("macos", "go") => Some("brew install go"),
        ("macos", "java") => Some("brew install openjdk"),
        ("macos", "ruby") => Some("brew install ruby"),
        ("macos", "docker") => Some("brew install --cask docker"),
        ("macos", "terraform") => Some("brew install terraform"),
        ("macos", "nix") => Some("sh <(curl -L https://nixos.org/nix/install)"),
        ("macos", "devcontainer") => Some("npm install --global @devcontainers/cli"),
        ("macos", "brew") => Some("https://brew.sh"),
        ("macos", "psql") => Some("brew install libpq"),
        ("macos", "redis-cli") => Some("brew install redis"),
        ("linux", "node" | "npm") => Some("sudo apt install nodejs npm"),
        ("linux", "pnpm") => Some("npm install --global pnpm"),
        ("linux", "yarn") => Some("npm install --global yarn"),
        ("linux", "rustc" | "cargo") => Some("sudo apt install rustc cargo"),
        ("linux", "python") => Some("sudo apt install python3"),
        ("linux", "go") => Some("sudo apt install golang"),
        ("linux", "java") => Some("sudo apt install default-jdk"),
        ("linux", "ruby") => Some("sudo apt install ruby"),
        ("linux", "docker") => Some("sudo apt install docker.io"),
        ("linux", "terraform") => Some("sudo apt install terraform"),
        ("linux", "nix") => Some("sh <(curl -L https://nixos.org/nix/install)"),
        ("linux", "devcontainer") => Some("npm install --global @devcontainers/cli"),
        ("linux", "brew") => Some("https://brew.sh"),
        ("linux", "psql") => Some("sudo apt install postgresql-client"),
        ("linux", "redis-cli") => Some("sudo apt install redis-tools"),
        ("windows", "node" | "npm") => Some("winget install OpenJS.NodeJS.LTS"),
        ("windows", "rustc" | "cargo") => Some("winget install Rustlang.Rustup"),
        ("windows", "python") => Some("winget install Python.Python.3.12"),
        ("windows", "go") => Some("winget install GoLang.Go"),
        ("windows", "java") => Some("winget install EclipseAdoptium.Temurin.21.JDK"),
        ("windows", "ruby") => Some("winget install RubyInstallerTeam.RubyWithDevKit"),
        ("windows", "docker") => Some("winget install Docker.DockerDesktop"),
        ("windows", "terraform") => Some("winget install Hashicorp.Terraform"),
        ("windows", "nix") => Some("https://nixos.org/download/"),
        ("windows", "devcontainer") => Some("npm install --global @devcontainers/cli"),
        _ => None,
    }
}

pub(crate) fn extract_version(text: &str) -> Option<String> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_owned)
}
pub(crate) fn normalize_exact(input: &str) -> String {
    format!(
        "={}",
        normalize_version(input.trim().trim_start_matches('v'))
    )
}

pub(crate) fn node_version_constraint(input: &str) -> String {
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

pub(crate) fn normalize_version(input: &str) -> String {
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
pub(crate) fn version_matches(found: &str, constraint: &str) -> bool {
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
