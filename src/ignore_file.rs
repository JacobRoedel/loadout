use std::{fs, path::Path};

use crate::check::matches_filter;
use crate::model::ResultItem;

/// Reads `.loadoutignore` from the repository root: one pattern per line,
/// blank lines and `#` comments ignored. This is a small, repository-local
/// list of intentional exceptions—not a place to configure how Loadout
/// behaves at runtime.
pub(crate) fn read_ignore_patterns(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join(".loadoutignore"))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// A pattern matches a `command`/`environment`/`dependency_state`/
/// `connectivity` kind or an exact check name, same as `--only`/`--skip`.
/// Patterns can also be scoped to a specific source with `name@source`,
/// where `source` matches as a substring (useful for ignoring one check in
/// one workspace member without ignoring it everywhere).
pub(crate) fn matches_ignore_pattern(item: &ResultItem, pattern: &str) -> bool {
    match pattern.split_once('@') {
        Some((name, source_substring)) => {
            item.name == name && item.source.contains(source_substring)
        }
        None => matches_filter(item, pattern),
    }
}

/// Drops results matching `.loadoutignore` and returns how many were
/// dropped, so callers can surface that count instead of silently hiding
/// findings.
pub(crate) fn apply_ignore_file(
    results: &mut Vec<ResultItem>,
    root: &Path,
    disabled: bool,
) -> usize {
    if disabled {
        return 0;
    }
    let patterns = read_ignore_patterns(root);
    if patterns.is_empty() {
        return 0;
    }
    let before = results.len();
    results.retain(|item| {
        !patterns
            .iter()
            .any(|pattern| matches_ignore_pattern(item, pattern))
    });
    before - results.len()
}
