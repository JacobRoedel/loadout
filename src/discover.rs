use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use walkdir::{DirEntry, WalkDir};

use crate::dotenv::env_requirements;
use crate::evaluate::{node_version_constraint, normalize_exact, normalize_version};
use crate::model::{Kind, Requirement, ResultItem, Status, command, display, warn};

pub(crate) fn ignored(entry: &DirEntry) -> bool {
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

pub(crate) fn discover(root: &Path, diagnostics: &mut Vec<ResultItem>) -> Vec<Requirement> {
    let mut found = Vec::new();
    let mut node_projects = Vec::new();
    let mut rust_projects = Vec::new();
    let mut python_projects = Vec::new();
    let mut ruby_projects = Vec::new();
    let mut node_workspace_roots = Vec::new();
    let mut cargo_workspace_roots = Vec::new();
    let mut python_workspace_roots = Vec::new();
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
                let dir = path.parent().unwrap().to_path_buf();
                if is_node_workspace_root(path) {
                    node_workspace_roots.push(dir.clone());
                }
                node_projects.push(dir);
                found.extend(node_requirements(path, diagnostics));
            }
            "pnpm-workspace.yaml" => {
                node_workspace_roots.push(path.parent().unwrap().to_path_buf());
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
                let dir = path.parent().unwrap().to_path_buf();
                if is_cargo_workspace_root(path) {
                    cargo_workspace_roots.push(dir.clone());
                }
                rust_projects.push(dir);
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
                let dir = path.parent().unwrap().to_path_buf();
                if is_uv_workspace_root(path) {
                    python_workspace_roots.push(dir.clone());
                }
                python_projects.push(dir);
                found.extend(pyproject_requirements(path));
            }
            "go.mod" => {
                found.push(command("go", go_mod_constraint(path), display(path), true));
            }
            ".tool-versions" => {
                found.extend(tool_versions_requirements(path));
            }
            "mise.toml" | ".mise.toml" => {
                found.extend(mise_requirements(path));
            }
            ".pre-commit-config.yaml" | ".pre-commit-config.yml" => {
                found.extend(pre_commit_requirements(path));
            }
            ".envrc" => {
                found.push(command("direnv", None, display(path), true));
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
            "Gemfile" => {
                ruby_projects.push(path.parent().unwrap().to_path_buf());
                found.push(command("ruby", None, display(path), true));
            }
            "Gemfile.lock" => {
                ruby_projects.push(path.parent().unwrap().to_path_buf());
                found.push(command("bundle", None, display(path), true));
            }
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
    rust_projects.sort();
    rust_projects.dedup();
    python_projects.sort();
    python_projects.dedup();
    ruby_projects.sort();
    ruby_projects.dedup();
    for path in node_projects {
        if is_workspace_member(&path, &node_workspace_roots) {
            continue;
        }
        let is_workspace_root = node_workspace_roots.contains(&path);
        node_dependency_warning(&mut found, &path, is_workspace_root);
    }
    for path in rust_projects {
        if is_workspace_member(&path, &cargo_workspace_roots) {
            continue;
        }
        dependency_warning(
            &mut found,
            &path,
            "target",
            "",
            "Cargo build artifacts are absent; run `cargo build`",
        );
    }
    for path in python_projects {
        if is_workspace_member(&path, &python_workspace_roots) {
            continue;
        }
        python_dependency_warning(&mut found, &path);
    }
    for path in ruby_projects {
        ruby_dependency_warning(&mut found, &path);
    }
    found.extend(env_requirements(root));
    found
}

/// True when `path` sits under one of `roots` but is not a root itself,
/// so a single check at the workspace root covers it.
pub(crate) fn is_workspace_member(path: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|root| path != root && path.starts_with(root))
}

pub(crate) fn is_node_workspace_root(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .is_some_and(|v| v.get("workspaces").is_some())
}

pub(crate) fn is_cargo_workspace_root(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
        .is_some_and(|v| v.get("workspace").is_some())
}

pub(crate) fn is_uv_workspace_root(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
        .is_some_and(|v| {
            v.get("tool")
                .and_then(|t| t.get("uv"))
                .and_then(|u| u.get("workspace"))
                .is_some()
        })
}

pub(crate) fn dependency_warning(
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

pub(crate) fn node_dependency_warning(
    found: &mut Vec<Requirement>,
    project: &Path,
    is_workspace_root: bool,
) {
    let install = if project.join("pnpm-lock.yaml").exists() {
        "pnpm install --frozen-lockfile"
    } else if project.join("yarn.lock").exists() {
        "yarn install --immutable"
    } else if project.join("bun.lock").exists() || project.join("bun.lockb").exists() {
        "bun install --frozen-lockfile"
    } else if project.join("package-lock.json").exists() {
        "npm ci"
    } else if project.join("pnpm-workspace.yaml").exists() {
        "pnpm install"
    } else {
        "npm install"
    };
    let location = if is_workspace_root {
        if project.join("turbo.json").exists() {
            " from the Turborepo workspace root"
        } else {
            " from the workspace root"
        }
    } else {
        ""
    };
    dependency_warning(
        found,
        project,
        "node_modules",
        ".pnp.cjs",
        &format!("Node dependencies do not appear to be installed; run `{install}`{location}"),
    );
}

/// Recommends the install command that matches whichever Python tool the
/// project has actually declared (uv, Poetry, Pipenv, a requirements file,
/// or a plain pyproject.toml), instead of a generic "activate a venv".
pub(crate) fn python_dependency_warning(found: &mut Vec<Requirement>, project: &Path) {
    let message = if project.join("uv.lock").exists() {
        "A local Python virtual environment is absent; run `uv sync` to create one and install dependencies".to_owned()
    } else if project.join("poetry.lock").exists() {
        "A local Python virtual environment is absent; run `poetry install` to create one and install dependencies".to_owned()
    } else if project.join("Pipfile.lock").exists() || project.join("Pipfile").exists() {
        "A local Python virtual environment is absent; run `pipenv install` to create one and install dependencies".to_owned()
    } else if let Some(requirements_file) = find_requirements_file(project) {
        format!(
            "A local Python virtual environment is absent; create one, activate it, and run `pip install -r {requirements_file}`"
        )
    } else if project.join("pyproject.toml").exists() {
        "A local Python virtual environment is absent; create one, activate it, and run `pip install -e .`".to_owned()
    } else {
        "A local Python virtual environment is absent".to_owned()
    };
    dependency_warning(found, project, ".venv", "venv", &message);
}

/// Finds a `requirements*.txt` file in `project`, preferring the
/// conventional `requirements.txt` over variants like
/// `requirements-dev.txt` when both exist.
pub(crate) fn find_requirements_file(project: &Path) -> Option<String> {
    let mut candidates: Vec<String> = fs::read_dir(project)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (name.starts_with("requirements") && (name.ends_with(".txt") || name == "requirements"))
                .then_some(name)
        })
        .collect();
    candidates.sort();
    if let Some(position) = candidates
        .iter()
        .position(|name| name == "requirements.txt")
    {
        return Some(candidates.remove(position));
    }
    candidates.into_iter().next()
}

/// Only warns when the project has explicitly opted into a local vendor
/// path (`.bundle/config`)—without that, gems installing to the system gem
/// home leave no repository-local trace we could check.
pub(crate) fn ruby_dependency_warning(found: &mut Vec<Requirement>, project: &Path) {
    if !project.join(".bundle").join("config").exists() {
        return;
    }
    dependency_warning(
        found,
        project,
        "vendor/bundle",
        "",
        "Ruby gems do not appear to be vendored; run `bundle install`",
    );
}

pub(crate) fn node_requirements(
    path: &Path,
    diagnostics: &mut Vec<ResultItem>,
) -> Vec<Requirement> {
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
        node_lockfile_health(path, name, diagnostics);
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

pub(crate) fn node_lockfile_health(
    path: &Path,
    declared_manager: &str,
    diagnostics: &mut Vec<ResultItem>,
) {
    let directory = path.parent().expect("package.json has a parent");
    let locks = [
        ("package-lock.json", "npm"),
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
    ];
    for (lockfile, manager) in locks {
        if directory.join(lockfile).exists() && manager != declared_manager {
            diagnostics.push(ResultItem {
                status: Status::Fail,
                kind: Kind::Command,
                name: declared_manager.into(),
                constraint: None,
                source: display(path),
                found: Some(lockfile.into()),
                message: format!("packageManager declares '{declared_manager}', but '{lockfile}' requires '{manager}'"),
            });
        }
    }
}

/// Maps a well-known asdf/mise plugin name to the command Loadout already
/// checks for that ecosystem. Unrecognized plugins are skipped rather than
/// guessed at, to avoid noisy checks for tools Loadout can't evaluate.
pub(crate) fn version_manager_tool_name(plugin: &str) -> Option<&'static str> {
    match plugin.to_lowercase().as_str() {
        "nodejs" | "node" => Some("node"),
        "python" | "python3" => Some("python"),
        "ruby" => Some("ruby"),
        "golang" | "go" => Some("go"),
        "rust" | "rustc" => Some("rustc"),
        "java" | "temurin" | "openjdk" | "adoptopenjdk" | "zulu" | "corretto" => Some("java"),
        "terraform" => Some("terraform"),
        "yarn" => Some("yarn"),
        "pnpm" => Some("pnpm"),
        "bun" => Some("bun"),
        _ => None,
    }
}

/// A pinned version like "system" or "latest" isn't a version Loadout can
/// check a local install against.
pub(crate) fn is_checkable_version(version: &str) -> bool {
    !matches!(version, "system" | "latest" | "" | "ref:master")
}

pub(crate) fn version_manager_requirement(
    tool: &str,
    version: &str,
    source: String,
) -> Vec<Requirement> {
    let constraint = version
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
        .then(|| normalize_exact(version));
    let mut found = vec![command(tool, constraint.clone(), source.clone(), true)];
    if tool == "rustc" {
        found.push(command("cargo", constraint, source, true));
    }
    found
}

/// Parses asdf's `.tool-versions`: one `<plugin> <version>` pair per line,
/// with `#` comments and multiple space-separated versions (asdf tries them
/// in order; Loadout checks the first) supported.
pub(crate) fn tool_versions_requirements(path: &Path) -> Vec<Requirement> {
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let line = line.split('#').next().unwrap_or("").trim();
            let mut parts = line.split_whitespace();
            let plugin = parts.next()?;
            let version = parts.next()?;
            let tool = version_manager_tool_name(plugin)?;
            is_checkable_version(version)
                .then(|| version_manager_requirement(tool, version, display(path)))
        })
        .flatten()
        .collect()
}

/// Parses mise's `[tools]` table from `mise.toml`/`.mise.toml`. A tool's
/// version may be a bare string, an array (mise tries them in order), or a
/// table with a `version` key.
pub(crate) fn mise_requirements(path: &Path) -> Vec<Requirement> {
    let Some(toml::Value::Table(tools)) = fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
        .and_then(|v| v.get("tools").cloned())
    else {
        return Vec::new();
    };
    tools
        .iter()
        .filter_map(|(plugin, version_value)| {
            let tool = version_manager_tool_name(plugin)?;
            let version = match version_value {
                toml::Value::String(s) => Some(s.clone()),
                toml::Value::Array(items) => {
                    items.first().and_then(|v| v.as_str()).map(str::to_owned)
                }
                toml::Value::Table(t) => {
                    t.get("version").and_then(|v| v.as_str()).map(str::to_owned)
                }
                _ => None,
            }?;
            is_checkable_version(&version)
                .then(|| version_manager_requirement(tool, &version, display(path)))
        })
        .flatten()
        .collect()
}

/// A `.pre-commit-config.yaml` requires the `pre-commit` tool itself, plus a
/// warning if hooks haven't been installed. `.git/hooks/pre-commit` is the
/// actual marker `pre-commit install` writes; it's only checked when the
/// config sits at a real git repository root, since that's the only place
/// the marker could exist.
pub(crate) fn pre_commit_requirements(path: &Path) -> Vec<Requirement> {
    let source = display(path);
    let mut found = vec![command("pre-commit", None, source.clone(), true)];
    let project = path.parent().expect("config file has a parent");
    if project.join(".git").is_dir() && !project.join(".git/hooks/pre-commit").exists() {
        found.push(Requirement {
            kind: Kind::DependencyState,
            name: "pre-commit hooks".into(),
            constraint: None,
            source,
            required: false,
            message: Some("Git hooks are not installed; run `pre-commit install`".into()),
        });
    }
    found
}

pub(crate) fn rust_toolchain_requirements(path: &Path) -> Vec<Requirement> {
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

pub(crate) fn cargo_requirements(path: &Path) -> Vec<Requirement> {
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

pub(crate) fn go_mod_constraint(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().and_then(|contents| {
        contents.lines().find_map(|line| {
            let version = line.trim().strip_prefix("go ")?.trim();
            (!version.is_empty()).then(|| format!(">={}", normalize_version(version)))
        })
    })
}

pub(crate) fn pyproject_requirements(path: &Path) -> Vec<Requirement> {
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
