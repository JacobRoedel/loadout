use std::{
    path::{Path, PathBuf},
    process::Command,
};

use clap::ValueEnum;

use crate::cli::Profile;
use crate::connectivity::connectivity_requirements;
use crate::discover::discover;
use crate::dotenv::read_local_env;
use crate::evaluate::{evaluate, parse_custom_requirement};
use crate::ignore_file::apply_ignore_file;
use crate::model::{CheckOptions, Report, Requirement, ResultItem, Status, command, kind_name};

/// Discovers requirements, expands profiles and one-off `--require` flags, and
/// evaluates every requirement against the local machine. Shared by `check`
/// and `doctor` so both commands see identical results.
pub(crate) fn gather_results(
    root: &Path,
    requirements: Vec<String>,
    profiles: Vec<Profile>,
    services: bool,
) -> Vec<ResultItem> {
    let mut diagnostics = Vec::new();
    let mut requirements_found = discover(root, &mut diagnostics);
    for profile in profiles {
        requirements_found.extend(profile_requirements(profile));
    }
    for input in requirements {
        match parse_custom_requirement(&input) {
            Ok(requirement) => requirements_found.push(requirement),
            Err(message) => {
                eprintln!("loadout: {message}");
                std::process::exit(2);
            }
        }
    }
    let env_values = read_local_env(root);
    if services {
        requirements_found.extend(connectivity_requirements(&requirements_found, &env_values));
    }
    let mut results: Vec<_> = requirements_found
        .iter()
        .map(|r| evaluate(r, root, &env_values))
        .collect();
    results.append(&mut diagnostics);
    results
}

pub(crate) fn run_check(root: PathBuf, options: CheckOptions) {
    let mut results = gather_results(
        &root,
        options.requirements,
        options.profiles,
        options.services,
    );
    let ignored = apply_ignore_file(&mut results, &root, options.no_ignore_file);
    results.retain(|item| {
        (options.only.is_empty()
            || options
                .only
                .iter()
                .any(|filter| matches_filter(item, filter)))
            && !options
                .skip
                .iter()
                .any(|filter| matches_filter(item, filter))
    });
    if let Some(base) = &options.changed {
        match changed_files(&root, base) {
            Ok(changed) => results.retain(|item| is_affected_by_change(&item.source, &changed)),
            Err(message) => {
                eprintln!("loadout: {message}");
                std::process::exit(2);
            }
        }
    }
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
        ignored,
        results,
    };
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("report serializes")
        );
    } else {
        print_human(&report, !options.no_color, options.quiet);
    }
    if options.annotate {
        print_annotations(&report, &root);
    }
    if report.failed > 0 || (options.strict && report.warnings > 0) {
        std::process::exit(1);
    }
}

/// Runs `git diff --name-only base...HEAD` and resolves each changed path to
/// an absolute path so it can be matched against requirement sources.
pub(crate) fn changed_files(
    root: &Path,
    base: &str,
) -> Result<std::collections::HashSet<PathBuf>, String> {
    let output = Command::new("git")
        .args(["diff", "--name-only", &format!("{base}...HEAD")])
        .current_dir(root)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git diff against '{base}' failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| root.join(line))
        .collect())
}

/// A requirement is affected by a set of changed files when its source is
/// one of those files, or (for directory-scoped sources like dependency
/// state) contains one of them. Sources that are not real paths on disk
/// (profiles, `--require`, `docker`, `aws`) can never be attributed to a
/// file change, so they are always kept.
pub(crate) fn is_affected_by_change(
    source: &str,
    changed: &std::collections::HashSet<PathBuf>,
) -> bool {
    let path = PathBuf::from(source);
    if !path.exists() {
        return true;
    }
    if changed.contains(&path) {
        return true;
    }
    path.is_dir() && changed.iter().any(|file| file.starts_with(&path))
}

/// Emits GitHub Actions workflow-command annotations (`::error`/`::warning`)
/// for every failing or warning result, so CI surfaces them inline on the
/// exact file that declared the requirement.
pub(crate) fn print_annotations(report: &Report, root: &Path) {
    for item in &report.results {
        let level = match item.status {
            Status::Fail => "error",
            Status::Warn => "warning",
            Status::Pass => continue,
        };
        let source_path = Path::new(&item.source);
        let file = source_path
            .strip_prefix(root)
            .map(|relative| relative.display().to_string())
            .unwrap_or_else(|_| item.source.clone());
        let file_attr = if source_path.is_file() {
            format!("file={},", annotation_escape(&file))
        } else {
            String::new()
        };
        let message = annotation_escape(&format!("{}: {}", item.name, item.message));
        println!("::{level} {file_attr}title=loadout::{message}");
    }
}

pub(crate) fn annotation_escape(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

pub(crate) fn profile_requirements(profile: Profile) -> Vec<Requirement> {
    let names: &[&str] = match profile {
        Profile::Web => &["node", "npm"],
        Profile::Rust => &["rustc", "cargo"],
        Profile::Python => &["python"],
        Profile::Containers => &["docker"],
        Profile::Infra => &["terraform"],
        Profile::Data => &["psql", "redis-cli"],
    };
    let profile_name = profile
        .to_possible_value()
        .expect("value enum has a name")
        .get_name()
        .to_owned();
    names
        .iter()
        .map(|name| command(name, None, format!("profile:{profile_name}"), true))
        .collect()
}

pub(crate) fn matches_filter(item: &ResultItem, filter: &str) -> bool {
    item.name == filter || kind_name(&item.kind) == filter
}

pub(crate) fn print_human(report: &Report, color: bool, quiet: bool) {
    if quiet && report.failed == 0 && report.warnings == 0 {
        return;
    }
    println!("Loadout: {}", report.path);
    for item in &report.results {
        if quiet && matches!(item.status, Status::Pass) {
            continue;
        }
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
    if report.ignored > 0 {
        println!(
            "{} check{} skipped via .loadoutignore",
            report.ignored,
            if report.ignored == 1 { "" } else { "s" }
        );
    }
}
