use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use crate::model::{Kind, Requirement, display};

/// Discovers required (and, where the file says so, optional) environment
/// variables from every `.env*.example`/`.env*.sample`/`.env*.template` file
/// at the repository root—covering not just `.env.example` but framework
/// and platform variants like `.env.development.example` or
/// `.env.local.example`.
pub(crate) fn env_requirements(root: &Path) -> Vec<Requirement> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut example_files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .map(|entry| entry.path())
        .filter(|path| is_env_example_file(path))
        .collect();
    example_files.sort();
    example_files
        .iter()
        .flat_map(|path| parse_env_example(&fs::read_to_string(path).unwrap_or_default(), path))
        .collect()
}

pub(crate) fn is_env_example_file(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    name.starts_with(".env")
        && (name.ends_with(".example") || name.ends_with(".sample") || name.ends_with(".template"))
}

/// A variable is treated as optional only when the file explicitly says
/// so—either a trailing `# optional` comment on its own line, or an
/// `# optional ...` comment on the line directly above it. Loadout never
/// infers "optional" just because an example value is present, since
/// placeholder values (`API_KEY=your-key-here`) are common for required
/// variables too.
pub(crate) fn parse_env_example(contents: &str, path: &Path) -> Vec<Requirement> {
    let source = display(path);
    let mut found = Vec::new();
    let mut preceding_comment_optional = false;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            preceding_comment_optional = comment.to_lowercase().contains("optional");
            continue;
        }
        let line = line.trim_start_matches("export ").trim();
        let Some((name, rest)) = line.split_once('=') else {
            preceding_comment_optional = false;
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            preceding_comment_optional = false;
            continue;
        }
        let inline_optional = rest
            .split_once('#')
            .is_some_and(|(_, comment)| comment.to_lowercase().contains("optional"));
        found.push(Requirement {
            kind: Kind::Environment,
            name: name.into(),
            constraint: None,
            source: source.clone(),
            required: !(preceding_comment_optional || inline_optional),
            message: None,
        });
        preceding_comment_optional = false;
    }
    found
}

pub(crate) fn read_local_env(root: &Path) -> HashMap<String, String> {
    let mut values = HashMap::new();
    // .envrc is a shell script, not a plain KEY=VALUE file, but its most
    // common lines (`export KEY=VALUE`) parse the same naive way as .env
    // does. Loadout does not evaluate shell expressions, so lines beyond
    // simple assignments (`use flake`, conditionals, `layout python`) are
    // ignored or, rarely, produce a harmless bogus key.
    for file in [
        ".env",
        ".env.local",
        ".env.development",
        ".env.development.local",
        ".env.test",
        ".env.test.local",
        ".env.production",
        ".env.production.local",
        ".envrc",
    ] {
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
