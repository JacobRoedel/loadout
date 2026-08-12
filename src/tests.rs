use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use crate::check::{
    annotation_escape, is_affected_by_change, matches_filter, profile_requirements,
};
use crate::cli::Profile;
use crate::connectivity::{connectivity_requirements, parse_service_url};
use crate::discover::{discover, go_mod_constraint};
use crate::discover::{
    is_cargo_workspace_root, is_node_workspace_root, is_uv_workspace_root, is_workspace_member,
    mise_requirements, node_dependency_warning, node_lockfile_health, pre_commit_requirements,
    python_dependency_warning, ruby_dependency_warning, tool_versions_requirements,
};
use crate::doctor::{doc_url_for, group_into_steps};
use crate::dotenv::{is_env_example_file, parse_env_example, read_local_env};
use crate::evaluate::{
    evaluate, extract_version, installation_hint_for, node_version_constraint,
    parse_custom_requirement, version_matches,
};
use crate::ignore_file::{apply_ignore_file, matches_ignore_pattern, read_ignore_patterns};
use crate::init::advisory_item;
use crate::model::{Kind, Requirement, ResultItem, Status, command};
use crate::scripts::{discover_scripts, just_recipes, make_targets, yaml_child_keys};

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
#[test]
fn detects_package_manager_and_lockfile_mismatch() {
    let directory = std::env::temp_dir().join(format!("loadout-lock-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let package = directory.join("package.json");
    fs::write(&package, "{}").unwrap();
    fs::write(directory.join("package-lock.json"), "{}").unwrap();
    let mut diagnostics = Vec::new();
    node_lockfile_health(&package, "pnpm", &mut diagnostics);
    assert_eq!(diagnostics.len(), 1);
    assert!(matches!(diagnostics[0].status, Status::Fail));
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn filters_match_kind_or_check_name() {
    let item = ResultItem {
        status: Status::Pass,
        kind: Kind::Environment,
        name: "DATABASE_URL".into(),
        constraint: None,
        source: "test".into(),
        found: None,
        message: "set".into(),
    };
    assert!(matches_filter(&item, "environment"));
    assert!(matches_filter(&item, "DATABASE_URL"));
    assert!(!matches_filter(&item, "command"));
}
#[test]
fn remediation_is_os_and_tool_specific() {
    assert_eq!(
        installation_hint_for("macos", "node"),
        Some("brew install node")
    );
    assert_eq!(
        installation_hint_for("windows", "terraform"),
        Some("winget install Hashicorp.Terraform")
    );
    assert_eq!(installation_hint_for("linux", "unknown"), None);
}
#[test]
fn profiles_expand_to_explicit_command_requirements() {
    let requirements = profile_requirements(Profile::Data);
    assert_eq!(requirements.len(), 2);
    assert_eq!(requirements[0].name, "psql");
    assert_eq!(requirements[0].source, "profile:data");
}
#[test]
fn detects_node_and_cargo_workspace_roots() {
    let directory =
        std::env::temp_dir().join(format!("loadout-workspace-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let package = directory.join("package.json");
    fs::write(&package, r#"{"workspaces": ["packages/*"]}"#).unwrap();
    assert!(is_node_workspace_root(&package));

    let plain_package = directory.join("plain.json");
    fs::write(&plain_package, "{}").unwrap();
    assert!(!is_node_workspace_root(&plain_package));

    let cargo_toml = directory.join("Cargo.toml");
    fs::write(&cargo_toml, "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    assert!(is_cargo_workspace_root(&cargo_toml));

    let plain_cargo = directory.join("plain-cargo.toml");
    fs::write(&plain_cargo, "[package]\nname = \"demo\"\n").unwrap();
    assert!(!is_cargo_workspace_root(&plain_cargo));

    let pyproject = directory.join("pyproject.toml");
    fs::write(
        &pyproject,
        "[tool.uv.workspace]\nmembers = [\"packages/*\"]\n",
    )
    .unwrap();
    assert!(is_uv_workspace_root(&pyproject));

    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn workspace_members_are_recognized_as_descendants_of_a_root() {
    let root = PathBuf::from("/repo");
    let member = PathBuf::from("/repo/packages/app");
    let roots = vec![root.clone()];
    assert!(is_workspace_member(&member, &roots));
    assert!(!is_workspace_member(&root, &roots));
    assert!(!is_workspace_member(
        &PathBuf::from("/other/project"),
        &roots
    ));
}
#[test]
fn workspace_root_install_message_names_the_root() {
    let directory = std::env::temp_dir().join(format!(
        "loadout-workspace-install-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("pnpm-workspace.yaml"),
        "packages:\n  - packages/*\n",
    )
    .unwrap();
    fs::write(directory.join("turbo.json"), "{}").unwrap();
    let mut found = Vec::new();
    node_dependency_warning(&mut found, &directory, true);
    assert_eq!(found.len(), 1);
    let message = found[0].message.as_deref().unwrap();
    assert!(message.contains("pnpm install"));
    assert!(message.contains("Turborepo workspace root"));
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn doctor_groups_blockers_in_remediation_order() {
    let results = vec![
        ResultItem {
            status: Status::Fail,
            kind: Kind::Environment,
            name: "DATABASE_URL".into(),
            constraint: None,
            source: "src".into(),
            found: None,
            message: "missing".into(),
        },
        ResultItem {
            status: Status::Pass,
            kind: Kind::Command,
            name: "cargo".into(),
            constraint: None,
            source: "src".into(),
            found: Some("1.0.0".into()),
            message: "ok".into(),
        },
        ResultItem {
            status: Status::Fail,
            kind: Kind::Command,
            name: "node".into(),
            constraint: None,
            source: "src".into(),
            found: None,
            message: "missing".into(),
        },
        ResultItem {
            status: Status::Warn,
            kind: Kind::DependencyState,
            name: "node_modules".into(),
            constraint: None,
            source: "src".into(),
            found: None,
            message: "run npm install".into(),
        },
    ];
    let steps = group_into_steps(&results);
    assert_eq!(steps.len(), 3);
    assert_eq!(
        steps[0].title,
        "Install missing tools and fix version mismatches"
    );
    assert_eq!(steps[0].items, vec![2]);
    assert_eq!(steps[1].title, "Install project dependencies");
    assert_eq!(steps[1].items, vec![3]);
    assert_eq!(steps[2].title, "Configure environment variables");
    assert_eq!(steps[2].items, vec![0]);
}
#[test]
fn doctor_docs_are_known_for_common_tools() {
    assert_eq!(doc_url_for("node"), Some("https://nodejs.org/en/download"));
    assert_eq!(
        doc_url_for("rustc"),
        Some("https://www.rust-lang.org/tools/install")
    );
    assert_eq!(doc_url_for("not-a-real-tool"), None);
}
#[test]
fn service_urls_are_parsed_without_leaking_credentials() {
    assert_eq!(
        parse_service_url("postgres://user:hunter2@db.internal:5433/app"),
        Some(("postgres", "db.internal".into(), 5433))
    );
    assert_eq!(
        parse_service_url("postgresql://db.internal/app"),
        Some(("postgres", "db.internal".into(), 5432))
    );
    assert_eq!(
        parse_service_url("redis://:pw@cache.internal/0"),
        Some(("redis", "cache.internal".into(), 6379))
    );
    assert_eq!(
        parse_service_url("mongodb://a.example.com,b.example.com/db"),
        Some(("mongodb", "a.example.com".into(), 27017))
    );
    assert_eq!(parse_service_url("not-a-url"), None);
    assert_eq!(parse_service_url("https://example.com"), None);
}
#[test]
fn service_url_parsing_never_returns_the_credential() {
    let (_, host, _) =
        parse_service_url("postgres://admin:s3cr3t@db.internal:5432/app").expect("parses");
    assert!(!host.contains("s3cr3t"));
    assert!(!host.contains("admin"));
}
#[test]
fn connectivity_requirements_add_docker_only_when_discovered() {
    let mut env_values = HashMap::new();
    env_values.insert(
        "DATABASE_URL".into(),
        "postgres://db.internal:5432/app".into(),
    );
    let discovered = vec![command("docker", None, "Dockerfile".into(), true)];
    let found = connectivity_requirements(&discovered, &env_values);
    assert!(found.iter().any(|r| r.name == "docker"));
    assert!(
        found
            .iter()
            .any(|r| r.name == "postgres" && r.constraint.as_deref() == Some("db.internal:5432"))
    );

    let without_docker = connectivity_requirements(&[], &env_values);
    assert!(!without_docker.iter().any(|r| r.name == "docker"));
}
#[test]
fn make_targets_skip_variables_and_special_targets() {
    let targets = make_targets(
        ".PHONY: build test\nVAR := value\n\nbuild:\n\tgo build ./...\n\ntest: build\n\tgo test ./...\n",
    );
    assert_eq!(targets, vec!["build", "test"]);
}
#[test]
fn just_recipes_skip_settings_and_attributes() {
    let recipes = just_recipes(
        "set shell := [\"bash\"]\n\n[private]\nlint:\n    cargo clippy\n\nbuild target=\"release\":\n    cargo build\n",
    );
    assert_eq!(recipes, vec!["lint", "build"]);
}
#[test]
fn yaml_child_keys_reads_immediate_children_only() {
    let compose = "version: \"3.8\"\nservices:\n  web:\n    image: nginx\n  db:\n    image: postgres\nvolumes:\n  data:\n";
    assert_eq!(yaml_child_keys(compose, "services"), vec!["web", "db"]);
    assert_eq!(yaml_child_keys(compose, "volumes"), vec!["data"]);
    assert_eq!(yaml_child_keys(compose, "networks"), Vec::<String>::new());
}
#[test]
fn discover_scripts_reads_npm_scripts() {
    let directory =
        std::env::temp_dir().join(format!("loadout-scripts-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("package.json"),
        r#"{"scripts": {"dev": "vite", "test": "vitest"}}"#,
    )
    .unwrap();
    let groups = discover_scripts(&directory);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].tool, "npm scripts");
    assert_eq!(groups[0].commands.len(), 2);
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn unattributable_sources_are_always_considered_changed() {
    let changed = std::collections::HashSet::new();
    assert!(is_affected_by_change("--require", &changed));
    assert!(is_affected_by_change("profile:web", &changed));
    assert!(is_affected_by_change("docker", &changed));
}
#[test]
fn file_sources_are_affected_only_when_changed() {
    let directory =
        std::env::temp_dir().join(format!("loadout-changed-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let touched = directory.join("package.json");
    let untouched = directory.join("Cargo.toml");
    fs::write(&touched, "{}").unwrap();
    fs::write(&untouched, "").unwrap();
    let mut changed = std::collections::HashSet::new();
    changed.insert(touched.clone());
    assert!(is_affected_by_change(touched.to_str().unwrap(), &changed));
    assert!(!is_affected_by_change(
        untouched.to_str().unwrap(),
        &changed
    ));
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn directory_sources_are_affected_when_they_contain_a_changed_file() {
    let directory =
        std::env::temp_dir().join(format!("loadout-changed-dir-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let changed_file = directory.join("src/lib.rs");
    let mut changed = std::collections::HashSet::new();
    changed.insert(changed_file);
    assert!(is_affected_by_change(directory.to_str().unwrap(), &changed));
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn annotation_messages_escape_percent_and_newlines() {
    assert_eq!(
        annotation_escape("100% done\nnext line"),
        "100%25 done%0Anext line"
    );
}
#[test]
fn ignore_patterns_match_kind_name_or_scoped_source() {
    let item = ResultItem {
        status: Status::Fail,
        kind: Kind::Environment,
        name: "DATABASE_URL".into(),
        constraint: None,
        source: "packages/api/.env.example".into(),
        found: None,
        message: "missing".into(),
    };
    assert!(matches_ignore_pattern(&item, "DATABASE_URL"));
    assert!(matches_ignore_pattern(&item, "environment"));
    assert!(matches_ignore_pattern(&item, "DATABASE_URL@packages/api"));
    assert!(!matches_ignore_pattern(&item, "DATABASE_URL@packages/web"));
    assert!(!matches_ignore_pattern(&item, "command"));
}
#[test]
fn ignore_file_comments_and_blank_lines_are_skipped() {
    let directory =
        std::env::temp_dir().join(format!("loadout-ignorefile-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(".loadoutignore"),
        "# a comment\n\nDATABASE_URL\n  \nconnectivity\n",
    )
    .unwrap();
    let patterns = read_ignore_patterns(&directory);
    assert_eq!(patterns, vec!["DATABASE_URL", "connectivity"]);
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn apply_ignore_file_can_be_disabled() {
    let directory = std::env::temp_dir().join(format!(
        "loadout-ignorefile-disable-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join(".loadoutignore"), "DATABASE_URL\n").unwrap();
    let mut results = vec![ResultItem {
        status: Status::Fail,
        kind: Kind::Environment,
        name: "DATABASE_URL".into(),
        constraint: None,
        source: "src".into(),
        found: None,
        message: "missing".into(),
    }];
    let ignored = apply_ignore_file(&mut results, &directory, true);
    assert_eq!(ignored, 0);
    assert_eq!(results.len(), 1);

    let ignored = apply_ignore_file(&mut results, &directory, false);
    assert_eq!(ignored, 1);
    assert!(results.is_empty());
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn python_install_guidance_matches_the_declared_tool() {
    let directory = std::env::temp_dir().join(format!(
        "loadout-python-install-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();

    fs::write(directory.join("uv.lock"), "").unwrap();
    let mut found = Vec::new();
    python_dependency_warning(&mut found, &directory);
    assert!(found[0].message.as_deref().unwrap().contains("uv sync"));
    fs::remove_file(directory.join("uv.lock")).unwrap();

    fs::write(directory.join("requirements-dev.txt"), "").unwrap();
    fs::write(directory.join("requirements.txt"), "").unwrap();
    let mut found = Vec::new();
    python_dependency_warning(&mut found, &directory);
    assert!(
        found[0]
            .message
            .as_deref()
            .unwrap()
            .contains("pip install -r requirements.txt")
    );
    fs::remove_dir_all(&directory).unwrap();
}
#[test]
fn ruby_dependency_warning_requires_local_bundle_config() {
    let directory =
        std::env::temp_dir().join(format!("loadout-ruby-install-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let mut found = Vec::new();
    ruby_dependency_warning(&mut found, &directory);
    assert!(found.is_empty(), "no .bundle/config means no signal");

    fs::create_dir_all(directory.join(".bundle")).unwrap();
    fs::write(directory.join(".bundle/config"), "").unwrap();
    let mut found = Vec::new();
    ruby_dependency_warning(&mut found, &directory);
    assert_eq!(found.len(), 1);
    assert!(
        found[0]
            .message
            .as_deref()
            .unwrap()
            .contains("bundle install")
    );
    fs::remove_dir_all(&directory).unwrap();
}
#[test]
fn env_example_file_recognition_covers_platform_variants() {
    for name in [
        ".env.example",
        ".env.sample",
        ".env.template",
        ".env.development.example",
        ".env.local.example",
        ".env.production.sample",
    ] {
        assert!(
            is_env_example_file(Path::new(name)),
            "{name} should be recognized"
        );
    }
    for name in [".env", ".env.local", ".env.development", "example.env"] {
        assert!(
            !is_env_example_file(Path::new(name)),
            "{name} should not be recognized"
        );
    }
}
#[test]
fn optional_variables_require_an_explicit_marker() {
    let contents = "DATABASE_URL=\n# Optional: has a sane default\nPORT=\nAPI_KEY=placeholder # optional\nPLACEHOLDER=fake-value-here\n";
    let requirements = parse_env_example(contents, Path::new(".env.example"));
    let required = |name: &str| {
        requirements
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .required
    };
    assert!(required("DATABASE_URL"));
    assert!(!required("PORT"));
    assert!(!required("API_KEY"));
    assert!(
        required("PLACEHOLDER"),
        "a present example value alone must not imply optional"
    );
}
#[test]
fn optional_environment_variables_warn_instead_of_fail() {
    let requirement = Requirement {
        kind: Kind::Environment,
        name: "PORT".into(),
        constraint: None,
        source: ".env.example".into(),
        required: false,
        message: None,
    };
    let result = evaluate(&requirement, Path::new("."), &HashMap::new());
    assert!(matches!(result.status, Status::Warn));
}
#[test]
fn tool_versions_maps_known_plugins_and_skips_unpinned() {
    let directory =
        std::env::temp_dir().join(format!("loadout-tool-versions-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(".tool-versions");
    fs::write(
        &path,
        "# comment\nnodejs 20.11.0\nruby system\nrust 1.75.0\nunknown-plugin 1.2.3\n",
    )
    .unwrap();
    let found = tool_versions_requirements(&path);
    let names: Vec<_> = found.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"node"));
    assert!(names.contains(&"rustc"));
    assert!(names.contains(&"cargo"), "rust plugin implies cargo too");
    assert!(!names.contains(&"ruby"), "system version is not checkable");
    assert_eq!(found.len(), 3);
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn mise_toml_reads_string_array_and_table_versions() {
    let directory = std::env::temp_dir().join(format!("loadout-mise-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("mise.toml");
    fs::write(
        &path,
        "[tools]\nnode = \"20.11.0\"\npython = [\"3.12.1\", \"3.11.0\"]\ngo = { version = \"1.22.0\" }\nterraform = \"latest\"\n",
    )
    .unwrap();
    let found = mise_requirements(&path);
    let names: Vec<_> = found.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"node"));
    assert!(names.contains(&"go"));
    assert!(!names.contains(&"terraform"), "latest is not checkable");
    let python = found.iter().find(|r| r.name == "python").unwrap();
    assert_eq!(python.constraint.as_deref(), Some("=3.12.1"));
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn pre_commit_hook_warning_requires_a_real_git_repo() {
    let directory =
        std::env::temp_dir().join(format!("loadout-pre-commit-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let config = directory.join(".pre-commit-config.yaml");
    fs::write(&config, "repos: []\n").unwrap();

    let found = pre_commit_requirements(&config);
    assert_eq!(found.len(), 1, "no .git means no hook-installed signal");
    assert_eq!(found[0].name, "pre-commit");

    fs::create_dir_all(directory.join(".git/hooks")).unwrap();
    let found = pre_commit_requirements(&config);
    assert_eq!(found.len(), 2);
    assert_eq!(found[1].name, "pre-commit hooks");

    fs::write(directory.join(".git/hooks/pre-commit"), "").unwrap();
    let found = pre_commit_requirements(&config);
    assert_eq!(found.len(), 1, "hook installed means no warning");

    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn envrc_values_are_readable_alongside_env_files() {
    let directory = std::env::temp_dir().join(format!("loadout-envrc-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(".envrc"),
        "export DATABASE_URL=postgres://localhost/dev\nuse flake\n",
    )
    .unwrap();
    let values = read_local_env(&directory);
    assert_eq!(
        values.get("DATABASE_URL").map(String::as_str),
        Some("postgres://localhost/dev")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn malformed_metadata_is_reported_with_its_source() {
    let directory =
        std::env::temp_dir().join(format!("loadout-parser-test-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("package.json"), "{ invalid").unwrap();
    fs::write(directory.join("Cargo.toml"), "[package\nname = ").unwrap();
    fs::write(directory.join("compose.yaml"), "services: [").unwrap();
    fs::write(directory.join(".env.example"), "DATABASE_URL\n").unwrap();
    fs::write(directory.join(".nvmrc"), "latest\n").unwrap();

    let mut diagnostics = Vec::new();
    discover(&directory, &mut diagnostics);
    assert!(diagnostics.len() >= 5, "every malformed file is surfaced");
    for name in [
        "package.json",
        "Cargo.toml",
        "compose.yaml",
        ".env.example",
        ".nvmrc",
    ] {
        assert!(
            diagnostics.iter().any(|item| item.source.ends_with(name)),
            "missing diagnostic for {name}"
        );
    }
    fs::remove_dir_all(directory).unwrap();
}
