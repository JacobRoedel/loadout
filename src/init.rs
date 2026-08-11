use std::path::PathBuf;

use crate::discover::discover;
use crate::model::{Advisory, AdvisoryItem, Kind, Requirement, display};
use crate::scripts::discover_scripts;

pub(crate) fn run_init(root: PathBuf, json: bool) {
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
        scripts: discover_scripts(&root),
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
    if !advisory.scripts.is_empty() {
        println!("\nRunnable commands detected in this repository:");
        for group in &advisory.scripts {
            println!("  {} ({})", group.tool, group.source);
            for command in &group.commands {
                println!("    {} — {}", command.name, command.run);
            }
        }
    }
    println!(
        "\nRun `loadout check` to validate this environment. Use `--require` only for one-off requirements."
    );
}

pub(crate) fn advisory_item(requirement: &Requirement) -> Option<AdvisoryItem> {
    let rendered = match requirement.kind {
        Kind::Command => match &requirement.constraint {
            Some(constraint) => format!("cmd:{}@{constraint}", requirement.name),
            None => format!("cmd:{}", requirement.name),
        },
        Kind::Environment => format!("env:{}", requirement.name),
        Kind::DependencyState | Kind::Connectivity => return None,
    };
    Some(AdvisoryItem {
        requirement: rendered,
        source: requirement.source.clone(),
    })
}
