use std::{env, path::PathBuf, process::Command};

use serde::Serialize;

use crate::check::gather_results;
use crate::ignore_file::apply_ignore_file;
use crate::model::{DoctorOptions, JSON_SCHEMA_VERSION, Kind, ResultItem, Status, display};

pub(crate) struct DoctorStep {
    pub(crate) title: &'static str,
    pub(crate) items: Vec<usize>,
}

/// Orders blockers into the sequence a developer should actually work
/// through: tools must exist before dependency installs can succeed, and
/// environment variables are typically the last thing configured.
pub(crate) fn group_into_steps(results: &[ResultItem]) -> Vec<DoctorStep> {
    let mut tools = Vec::new();
    let mut deps = Vec::new();
    let mut envs = Vec::new();
    let mut services = Vec::new();
    for (index, item) in results.iter().enumerate() {
        if matches!(item.status, Status::Pass) {
            continue;
        }
        match item.kind {
            Kind::Command => tools.push(index),
            Kind::DependencyState => deps.push(index),
            Kind::Environment => envs.push(index),
            Kind::Connectivity => services.push(index),
        }
    }
    [
        ("Install missing tools and fix version mismatches", tools),
        ("Install project dependencies", deps),
        ("Configure environment variables", envs),
        ("Check service connectivity", services),
    ]
    .into_iter()
    .filter(|(_, items)| !items.is_empty())
    .map(|(title, items)| DoctorStep { title, items })
    .collect()
}

pub(crate) fn doc_url_for(name: &str) -> Option<&'static str> {
    match name {
        "node" | "npm" => Some("https://nodejs.org/en/download"),
        "pnpm" => Some("https://pnpm.io/installation"),
        "yarn" => Some("https://yarnpkg.com/getting-started/install"),
        "bun" => Some("https://bun.sh/docs/installation"),
        "rustc" | "cargo" => Some("https://www.rust-lang.org/tools/install"),
        "python" | "python3" => Some("https://www.python.org/downloads/"),
        "uv" => Some("https://docs.astral.sh/uv/getting-started/installation/"),
        "poetry" => Some("https://python-poetry.org/docs/#installation"),
        "pipenv" => Some("https://pipenv.pypa.io/en/latest/installation.html"),
        "go" => Some("https://go.dev/doc/install"),
        "java" => Some("https://adoptium.net/installation/"),
        "ruby" => Some("https://www.ruby-lang.org/en/documentation/installation/"),
        "bundle" => Some("https://bundler.io/#getting-started"),
        "docker" => Some("https://docs.docker.com/get-docker/"),
        "terraform" => Some("https://developer.hashicorp.com/terraform/install"),
        "psql" => Some("https://www.postgresql.org/download/"),
        "redis-cli" => Some("https://redis.io/docs/getting-started/installation/"),
        _ => None,
    }
}

/// Opens `url` with the OS's default handler. Never modifies the machine; a
/// failure to launch a browser is reported but not fatal.
fn open_url(url: &str) {
    let result = match env::consts::OS {
        "macos" => Command::new("open").arg(url).status(),
        "windows" => Command::new("cmd").args(["/C", "start", "", url]).status(),
        _ => Command::new("xdg-open").arg(url).status(),
    };
    if !result.is_ok_and(|status| status.success()) {
        eprintln!("loadout: could not open {url} automatically; open it manually");
    }
}

#[derive(Serialize)]
struct DoctorReport {
    schema_version: &'static str,
    path: String,
    passed: usize,
    ignored: usize,
    steps: Vec<DoctorStepReport>,
}

#[derive(Serialize)]
struct DoctorStepReport {
    title: String,
    items: Vec<DoctorItemReport>,
}

#[derive(Serialize)]
struct DoctorItemReport {
    status: Status,
    kind: Kind,
    name: String,
    constraint: Option<String>,
    source: String,
    found: Option<String>,
    message: String,
    docs: Option<String>,
}

pub(crate) fn run_doctor(root: PathBuf, options: DoctorOptions) {
    let mut results = gather_results(
        &root,
        options.requirements,
        options.profiles,
        options.services,
    );
    let ignored = apply_ignore_file(&mut results, &root, options.no_ignore_file);
    results.sort_by(|a, b| a.name.cmp(&b.name).then(a.source.cmp(&b.source)));
    let passed = results
        .iter()
        .filter(|r| matches!(r.status, Status::Pass))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, Status::Fail))
        .count();
    let steps = group_into_steps(&results);

    if options.json {
        let report = DoctorReport {
            schema_version: JSON_SCHEMA_VERSION,
            path: root.display().to_string(),
            passed,
            ignored,
            steps: steps
                .iter()
                .map(|step| DoctorStepReport {
                    title: step.title.into(),
                    items: step
                        .items
                        .iter()
                        .map(|&index| {
                            let item = &results[index];
                            DoctorItemReport {
                                status: item.status,
                                kind: item.kind.clone(),
                                name: item.name.clone(),
                                constraint: item.constraint.clone(),
                                source: item.source.clone(),
                                found: item.found.clone(),
                                message: item.message.clone(),
                                docs: doc_url_for(&item.name).map(str::to_owned),
                            }
                        })
                        .collect(),
                })
                .collect(),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("report serializes")
        );
        if failed > 0 {
            std::process::exit(1);
        }
        return;
    }

    println!("Loadout doctor: {}", display(&root));
    if steps.is_empty() {
        println!(
            "\nNo blockers found. All {passed} checks passed — this repository looks ready for local development."
        );
        if ignored > 0 {
            println!(
                "{ignored} check{} skipped via .loadoutignore",
                if ignored == 1 { "" } else { "s" }
            );
        }
        return;
    }
    let mut opened = std::collections::HashSet::new();
    for (step_number, step) in steps.iter().enumerate() {
        println!("\nStep {} — {}", step_number + 1, step.title);
        for (item_number, &index) in step.items.iter().enumerate() {
            let item = &results[index];
            let label = match item.status {
                Status::Fail => "FAIL",
                Status::Warn => "WARN",
                Status::Pass => unreachable!("passing checks are excluded from steps"),
            };
            let label = if options.no_color {
                label.to_owned()
            } else {
                let code = if matches!(item.status, Status::Fail) {
                    "31"
                } else {
                    "33"
                };
                format!("\x1b[{code}m{label}\x1b[0m")
            };
            println!(
                "  {}. [{label}] {} — {}",
                item_number + 1,
                item.name,
                item.message
            );
            println!("     Source: {}", item.source);
            if let Some(url) = doc_url_for(&item.name) {
                if options.open_docs {
                    if opened.insert(item.name.clone()) {
                        println!("     Opening docs: {url}");
                        open_url(url);
                    }
                } else {
                    println!("     Docs: {url}");
                }
            }
        }
    }
    println!(
        "\n{passed} checks already pass. Loadout only reports findings; apply the steps above yourself."
    );
    if ignored > 0 {
        println!(
            "{ignored} check{} skipped via .loadoutignore",
            if ignored == 1 { "" } else { "s" }
        );
    }
    if failed > 0 {
        std::process::exit(1);
    }
}
