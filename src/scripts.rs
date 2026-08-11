use std::{fs, path::Path};

use serde_json::Value;
use walkdir::WalkDir;

use crate::discover::ignored;
use crate::model::{ScriptCommand, ScriptGroup, display};

/// Reads package.json scripts, Makefiles, Justfiles, Taskfiles, and Docker
/// Compose files to surface the commands a developer would actually run for
/// dev/test/build—advisory only, nothing here is validated or executed.
pub(crate) fn discover_scripts(root: &Path) -> Vec<ScriptGroup> {
    let mut groups = Vec::new();
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
        let contents = || fs::read_to_string(path).unwrap_or_default();
        match name.as_ref() {
            "package.json" => {
                if let Ok(value) = serde_json::from_str::<Value>(&contents())
                    && let Some(scripts) = value.get("scripts").and_then(Value::as_object)
                {
                    let mut commands: Vec<_> = scripts
                        .iter()
                        .filter_map(|(name, run)| {
                            Some(ScriptCommand {
                                name: name.clone(),
                                run: run.as_str()?.to_owned(),
                            })
                        })
                        .collect();
                    commands.sort_by(|a, b| a.name.cmp(&b.name));
                    if !commands.is_empty() {
                        groups.push(ScriptGroup {
                            tool: "npm scripts",
                            source: display(path),
                            commands,
                        });
                    }
                }
            }
            "Makefile" | "makefile" | "GNUmakefile" => {
                let commands: Vec<_> = make_targets(&contents())
                    .into_iter()
                    .map(|target| ScriptCommand {
                        run: format!("make {target}"),
                        name: target,
                    })
                    .collect();
                if !commands.is_empty() {
                    groups.push(ScriptGroup {
                        tool: "Makefile",
                        source: display(path),
                        commands,
                    });
                }
            }
            "Justfile" | "justfile" | ".justfile" => {
                let commands: Vec<_> = just_recipes(&contents())
                    .into_iter()
                    .map(|recipe| ScriptCommand {
                        run: format!("just {recipe}"),
                        name: recipe,
                    })
                    .collect();
                if !commands.is_empty() {
                    groups.push(ScriptGroup {
                        tool: "Justfile",
                        source: display(path),
                        commands,
                    });
                }
            }
            "Taskfile.yml" | "Taskfile.yaml" => {
                let commands: Vec<_> = yaml_child_keys(&contents(), "tasks")
                    .into_iter()
                    .map(|task| ScriptCommand {
                        run: format!("task {task}"),
                        name: task,
                    })
                    .collect();
                if !commands.is_empty() {
                    groups.push(ScriptGroup {
                        tool: "Taskfile",
                        source: display(path),
                        commands,
                    });
                }
            }
            "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => {
                let commands: Vec<_> = yaml_child_keys(&contents(), "services")
                    .into_iter()
                    .map(|service| ScriptCommand {
                        run: format!("docker compose up {service}"),
                        name: service,
                    })
                    .collect();
                if !commands.is_empty() {
                    groups.push(ScriptGroup {
                        tool: "Docker Compose",
                        source: display(path),
                        commands,
                    });
                }
            }
            _ => {}
        }
    }
    groups
}

/// Extracts top-level Makefile target names, skipping variable assignments,
/// special targets (`.PHONY`), and pattern rules.
pub(crate) fn make_targets(contents: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for line in contents.lines() {
        if line.starts_with([' ', '\t']) || line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name_part, rest)) = line.split_once(':') else {
            continue;
        };
        if rest.starts_with('=') {
            continue;
        }
        let name = name_part.trim();
        if name.is_empty() || name.starts_with('.') || name.contains(['$', ' ', '%']) {
            continue;
        }
        if !targets.iter().any(|t| t == name) {
            targets.push(name.to_owned());
        }
    }
    targets
}

/// Extracts top-level Justfile recipe names, skipping settings, imports, and
/// attributes.
pub(crate) fn just_recipes(contents: &str) -> Vec<String> {
    let mut recipes = Vec::new();
    for line in contents.lines() {
        if line.starts_with([' ', '\t']) || line.trim().is_empty() || line.starts_with(['#', '[']) {
            continue;
        }
        let Some(colon_index) = line.find(':') else {
            continue;
        };
        let name = line[..colon_index].split_whitespace().next().unwrap_or("");
        if name.is_empty()
            || name.starts_with('@')
            || matches!(name, "set" | "import" | "mod" | "export")
        {
            continue;
        }
        if !recipes.iter().any(|r| r == name) {
            recipes.push(name.to_owned());
        }
    }
    recipes
}

/// Best-effort YAML scan for the immediate child keys of a top-level
/// section (e.g. `services:` or `tasks:`), without pulling in a YAML parser.
pub(crate) fn yaml_child_keys(contents: &str, section: &str) -> Vec<String> {
    let mut in_section = false;
    let mut section_indent = None;
    let mut keys = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_section {
            if indent == 0 && trimmed.trim_end() == format!("{section}:") {
                in_section = true;
            }
            continue;
        }
        if indent == 0 {
            break;
        }
        match section_indent {
            None => section_indent = Some(indent),
            Some(expected) if indent != expected => continue,
            Some(_) => {}
        }
        if let Some((key, _)) = trimmed.split_once(':') {
            let key = key.trim().trim_matches(['"', '\'']);
            if !key.is_empty() {
                keys.push(key.to_owned());
            }
        }
    }
    keys
}
