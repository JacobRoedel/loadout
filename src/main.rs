use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, shells};
use clap_mangen::Man;
use semver::{Version, VersionReq};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    env, fs, io,
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
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
        /// Add a reusable built-in requirement profile (also reads LOADOUT_PROFILE)
        #[arg(long, value_enum, value_delimiter = ',', env = "LOADOUT_PROFILE")]
        profile: Vec<Profile>,
        /// Emit a machine-readable report
        #[arg(long)]
        json: bool,
        /// Disable ANSI colors in human output
        #[arg(long)]
        no_color: bool,
        /// Include only checks matching a kind (command, environment, dependency_state, connectivity) or name
        #[arg(long, value_name = "FILTER")]
        only: Vec<String>,
        /// Exclude checks matching a kind (command, environment, dependency_state, connectivity) or name
        #[arg(long, value_name = "FILTER")]
        skip: Vec<String>,
        /// Treat warnings as failures for this invocation
        #[arg(long)]
        strict: bool,
        /// Suppress passing checks in human output
        #[arg(long, conflicts_with = "json")]
        quiet: bool,
        /// Also verify configured services are reachable (makes network connections; opt-in)
        #[arg(long)]
        services: bool,
        /// Only check requirements whose source file changed relative to this git ref (e.g. origin/main)
        #[arg(long, value_name = "BASE_REF")]
        changed: Option<String>,
        /// Emit GitHub Actions ::error/::warning annotations for failing and warning checks
        #[arg(long)]
        annotate: bool,
        /// Ignore the repository's .loadoutignore file for this invocation
        #[arg(long)]
        no_ignore_file: bool,
    },
    /// Print an advisory summary of detected requirements without creating files
    Init {
        /// Repository directory (defaults to the current directory)
        path: Option<PathBuf>,
        /// Emit a machine-readable advisory
        #[arg(long)]
        json: bool,
    },
    /// Guided view that groups blockers into ordered next steps
    Doctor {
        /// Repository directory (defaults to the current directory)
        path: Option<PathBuf>,
        /// Add a one-off requirement: cmd:NAME, cmd:NAME@VERSION, or env:NAME
        #[arg(long = "require", value_name = "REQUIREMENT")]
        requirements: Vec<String>,
        /// Add a reusable built-in requirement profile (also reads LOADOUT_PROFILE)
        #[arg(long, value_enum, value_delimiter = ',', env = "LOADOUT_PROFILE")]
        profile: Vec<Profile>,
        /// Emit a machine-readable report
        #[arg(long)]
        json: bool,
        /// Disable ANSI colors in human output
        #[arg(long)]
        no_color: bool,
        /// Open each missing tool's install docs in your browser
        #[arg(long)]
        open_docs: bool,
        /// Also verify configured services are reachable (makes network connections; opt-in)
        #[arg(long)]
        services: bool,
        /// Ignore the repository's .loadoutignore file for this invocation
        #[arg(long)]
        no_ignore_file: bool,
    },
    /// Generate shell completion scripts to stdout
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Generate a roff man page to stdout
    Man,
}

#[derive(Clone, Debug, ValueEnum)]
enum Shell {
    Bash,
    Elvish,
    Fish,
    Powershell,
    Zsh,
}

#[derive(Clone, Debug, ValueEnum)]
enum Profile {
    Web,
    Rust,
    Python,
    Containers,
    Infra,
    Data,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Command,
    Environment,
    DependencyState,
    Connectivity,
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
    ignored: usize,
}

struct CheckOptions {
    requirements: Vec<String>,
    profiles: Vec<Profile>,
    json: bool,
    no_color: bool,
    only: Vec<String>,
    skip: Vec<String>,
    strict: bool,
    quiet: bool,
    services: bool,
    changed: Option<String>,
    annotate: bool,
    no_ignore_file: bool,
}

struct DoctorOptions {
    requirements: Vec<String>,
    profiles: Vec<Profile>,
    json: bool,
    no_color: bool,
    open_docs: bool,
    services: bool,
    no_ignore_file: bool,
}

#[derive(Serialize)]
struct Advisory {
    path: String,
    writes_files: bool,
    requirements: Vec<AdvisoryItem>,
    scripts: Vec<ScriptGroup>,
}

#[derive(Serialize)]
struct AdvisoryItem {
    requirement: String,
    source: String,
}

#[derive(Serialize)]
struct ScriptGroup {
    tool: &'static str,
    source: String,
    commands: Vec<ScriptCommand>,
}

#[derive(Serialize)]
struct ScriptCommand {
    name: String,
    run: String,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Check {
            path,
            requirements,
            profile,
            json,
            no_color,
            only,
            skip,
            strict,
            quiet,
            services,
            changed,
            annotate,
            no_ignore_file,
        }) => run_check(
            resolve_root(path),
            CheckOptions {
                requirements,
                profiles: profile,
                json,
                no_color,
                only,
                skip,
                strict,
                quiet,
                services,
                changed,
                annotate,
                no_ignore_file,
            },
        ),
        Some(Commands::Init { path, json }) => run_init(resolve_root(path), json),
        Some(Commands::Doctor {
            path,
            requirements,
            profile,
            json,
            no_color,
            open_docs,
            services,
            no_ignore_file,
        }) => run_doctor(
            resolve_root(path),
            DoctorOptions {
                requirements,
                profiles: profile,
                json,
                no_color,
                open_docs,
                services,
                no_ignore_file,
            },
        ),
        Some(Commands::Completions { shell }) => run_completions(shell),
        Some(Commands::Man) => run_man(),
        None => {
            let mut cmd = Cli::command();
            cmd.print_help().expect("stdout is available");
            println!();
        }
    }
}

fn run_completions(shell: Shell) {
    let mut command = Cli::command();
    let mut output = io::stdout();
    match shell {
        Shell::Bash => generate(shells::Bash, &mut command, "loadout", &mut output),
        Shell::Elvish => generate(shells::Elvish, &mut command, "loadout", &mut output),
        Shell::Fish => generate(shells::Fish, &mut command, "loadout", &mut output),
        Shell::Powershell => generate(shells::PowerShell, &mut command, "loadout", &mut output),
        Shell::Zsh => generate(shells::Zsh, &mut command, "loadout", &mut output),
    }
}

fn run_man() {
    let command = Cli::command();
    Man::new(command)
        .render(&mut io::stdout())
        .expect("stdout is available");
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

/// Discovers requirements, expands profiles and one-off `--require` flags, and
/// evaluates every requirement against the local machine. Shared by `check`
/// and `doctor` so both commands see identical results.
fn gather_results(
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

fn run_check(root: PathBuf, options: CheckOptions) {
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
fn changed_files(root: &Path, base: &str) -> Result<std::collections::HashSet<PathBuf>, String> {
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
fn is_affected_by_change(source: &str, changed: &std::collections::HashSet<PathBuf>) -> bool {
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
fn print_annotations(report: &Report, root: &Path) {
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

fn annotation_escape(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

struct DoctorStep {
    title: &'static str,
    items: Vec<usize>,
}

/// Orders blockers into the sequence a developer should actually work
/// through: tools must exist before dependency installs can succeed, and
/// environment variables are typically the last thing configured.
fn group_into_steps(results: &[ResultItem]) -> Vec<DoctorStep> {
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

fn doc_url_for(name: &str) -> Option<&'static str> {
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

fn run_doctor(root: PathBuf, options: DoctorOptions) {
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

fn profile_requirements(profile: Profile) -> Vec<Requirement> {
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

fn matches_filter(item: &ResultItem, filter: &str) -> bool {
    item.name == filter || kind_name(&item.kind) == filter
}

/// Reads `.loadoutignore` from the repository root: one pattern per line,
/// blank lines and `#` comments ignored. This is a small, repository-local
/// list of intentional exceptions—not a place to configure how Loadout
/// behaves at runtime.
fn read_ignore_patterns(root: &Path) -> Vec<String> {
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
fn matches_ignore_pattern(item: &ResultItem, pattern: &str) -> bool {
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
fn apply_ignore_file(results: &mut Vec<ResultItem>, root: &Path, disabled: bool) -> usize {
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

fn kind_name(kind: &Kind) -> &str {
    match kind {
        Kind::Command => "command",
        Kind::Environment => "environment",
        Kind::DependencyState => "dependency_state",
        Kind::Connectivity => "connectivity",
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

fn advisory_item(requirement: &Requirement) -> Option<AdvisoryItem> {
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

/// Reads package.json scripts, Makefiles, Justfiles, Taskfiles, and Docker
/// Compose files to surface the commands a developer would actually run for
/// dev/test/build—advisory only, nothing here is validated or executed.
fn discover_scripts(root: &Path) -> Vec<ScriptGroup> {
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
fn make_targets(contents: &str) -> Vec<String> {
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
fn just_recipes(contents: &str) -> Vec<String> {
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
fn yaml_child_keys(contents: &str, section: &str) -> Vec<String> {
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

fn discover(root: &Path, diagnostics: &mut Vec<ResultItem>) -> Vec<Requirement> {
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
fn is_workspace_member(path: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|root| path != root && path.starts_with(root))
}

fn is_node_workspace_root(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .is_some_and(|v| v.get("workspaces").is_some())
}

fn is_cargo_workspace_root(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
        .is_some_and(|v| v.get("workspace").is_some())
}

fn is_uv_workspace_root(path: &Path) -> bool {
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

fn node_dependency_warning(found: &mut Vec<Requirement>, project: &Path, is_workspace_root: bool) {
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
fn python_dependency_warning(found: &mut Vec<Requirement>, project: &Path) {
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
fn find_requirements_file(project: &Path) -> Option<String> {
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
fn ruby_dependency_warning(found: &mut Vec<Requirement>, project: &Path) {
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

fn node_lockfile_health(path: &Path, declared_manager: &str, diagnostics: &mut Vec<ResultItem>) {
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

/// Discovers required (and, where the file says so, optional) environment
/// variables from every `.env*.example`/`.env*.sample`/`.env*.template` file
/// at the repository root—covering not just `.env.example` but framework
/// and platform variants like `.env.development.example` or
/// `.env.local.example`.
fn env_requirements(root: &Path) -> Vec<Requirement> {
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

fn is_env_example_file(path: &Path) -> bool {
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
fn parse_env_example(contents: &str, path: &Path) -> Vec<Requirement> {
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

fn read_local_env(root: &Path) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for file in [
        ".env",
        ".env.local",
        ".env.development",
        ".env.development.local",
        ".env.test",
        ".env.test.local",
        ".env.production",
        ".env.production.local",
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

/// Builds opt-in connectivity checks from already-discovered requirements and
/// the resolved environment: reachable database/cache URLs, a running Docker
/// daemon (if Docker is used by the repository), and AWS identity (if the
/// AWS CLI is configured). Only invoked when the caller explicitly asks for
/// network connectivity checks.
fn connectivity_requirements(
    discovered: &[Requirement],
    env_values: &HashMap<String, String>,
) -> Vec<Requirement> {
    let mut found = Vec::new();
    let mut combined = env_values.clone();
    for (key, value) in env::vars() {
        combined.insert(key, value);
    }
    let mut seen = std::collections::HashSet::new();
    let mut names: Vec<_> = combined.keys().cloned().collect();
    names.sort();
    for name in names {
        let value = &combined[&name];
        let Some((service, host, port)) = parse_service_url(value) else {
            continue;
        };
        let target = format!("{service}:{host}:{port}");
        if !seen.insert(target) {
            continue;
        }
        found.push(Requirement {
            kind: Kind::Connectivity,
            name: service.into(),
            constraint: Some(format!("{host}:{port}")),
            source: format!("env:{name}"),
            required: false,
            message: None,
        });
    }
    if discovered
        .iter()
        .any(|r| r.kind == Kind::Command && r.name == "docker")
    {
        found.push(Requirement {
            kind: Kind::Connectivity,
            name: "docker".into(),
            constraint: None,
            source: "docker".into(),
            required: false,
            message: None,
        });
    }
    if aws_configured() {
        found.push(Requirement {
            kind: Kind::Connectivity,
            name: "aws".into(),
            constraint: None,
            source: "aws".into(),
            required: false,
            message: None,
        });
    }
    found
}

/// Recognizes common database/cache/queue connection strings and extracts
/// only the host and port—credentials and paths are discarded immediately.
fn parse_service_url(value: &str) -> Option<(&'static str, String, u16)> {
    const SCHEMES: &[(&str, &str, u16)] = &[
        ("postgres://", "postgres", 5432),
        ("postgresql://", "postgres", 5432),
        ("mysql://", "mysql", 3306),
        ("rediss://", "redis", 6380),
        ("redis://", "redis", 6379),
        ("mongodb://", "mongodb", 27017),
        ("amqp://", "rabbitmq", 5672),
    ];
    let (service, default_port, rest) = SCHEMES.iter().find_map(|(prefix, service, port)| {
        value
            .strip_prefix(prefix)
            .map(|rest| (*service, *port, rest))
    })?;
    let after_at = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    let host_port = after_at
        .split(['/', '?'])
        .next()
        .unwrap_or(after_at)
        .split(',')
        .next()
        .unwrap_or(after_at);
    if host_port.is_empty() {
        return None;
    }
    let (host, port) = host_port
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|port| (host.to_owned(), port)))
        .unwrap_or((host_port.to_owned(), default_port));
    (!host.is_empty()).then_some((service, host, port))
}

fn aws_configured() -> bool {
    let cli_available = Command::new("aws")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !cli_available {
        return false;
    }
    env::var("AWS_PROFILE").is_ok()
        || env::var("AWS_ACCESS_KEY_ID").is_ok()
        || home_dir().is_some_and(|home| {
            home.join(".aws/credentials").exists() || home.join(".aws/config").exists()
        })
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
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

/// Verifies a configured service is reachable. Only ever reports pass/warn
/// status and, for network targets, the host:port that was dialed—never a
/// credential, connection string, or command output.
fn evaluate_connectivity(requirement: &Requirement) -> ResultItem {
    match requirement.name.as_str() {
        "docker" => evaluate_docker_daemon(requirement),
        "aws" => evaluate_aws_identity(requirement),
        service => evaluate_tcp_reachable(service, requirement),
    }
}

fn evaluate_tcp_reachable(service: &str, requirement: &Requirement) -> ResultItem {
    let target = requirement.constraint.clone().unwrap_or_default();
    let reachable = target
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .is_some_and(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok());
    ResultItem {
        status: if reachable {
            Status::Pass
        } else {
            Status::Warn
        },
        kind: Kind::Connectivity,
        name: service.into(),
        constraint: requirement.constraint.clone(),
        source: requirement.source.clone(),
        found: reachable.then(|| "reachable".into()),
        message: if reachable {
            format!("{service} is reachable")
        } else {
            format!("Could not reach {service} at the configured host and port")
        },
    }
}

fn evaluate_docker_daemon(requirement: &Requirement) -> ResultItem {
    let reachable = Command::new("docker")
        .arg("info")
        .output()
        .is_ok_and(|output| output.status.success());
    ResultItem {
        status: if reachable {
            Status::Pass
        } else {
            Status::Warn
        },
        kind: Kind::Connectivity,
        name: "docker".into(),
        constraint: None,
        source: requirement.source.clone(),
        found: reachable.then(|| "reachable".into()),
        message: if reachable {
            "Docker daemon is reachable".into()
        } else {
            "Docker daemon is not reachable; is Docker running?".into()
        },
    }
}

fn evaluate_aws_identity(requirement: &Requirement) -> ResultItem {
    let reachable = Command::new("aws")
        .args(["sts", "get-caller-identity", "--output", "text"])
        .output()
        .is_ok_and(|output| output.status.success());
    ResultItem {
        status: if reachable {
            Status::Pass
        } else {
            Status::Warn
        },
        kind: Kind::Connectivity,
        name: "aws".into(),
        constraint: None,
        source: requirement.source.clone(),
        found: reachable.then(|| "reachable".into()),
        message: if reachable {
            "AWS credentials are valid and the identity endpoint is reachable".into()
        } else {
            "Could not verify AWS identity; check credentials and network access".into()
        },
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

fn missing_command_message(name: &str) -> String {
    let generic = format!("Install '{name}' and ensure it is available on PATH");
    installation_hint_for(env::consts::OS, name)
        .map(|hint| format!("{generic}; try `{hint}`"))
        .unwrap_or(generic)
}

fn installation_hint_for(os: &str, name: &str) -> Option<&'static str> {
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
        _ => None,
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
fn print_human(report: &Report, color: bool, quiet: bool) {
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
    #[test]
    fn detects_package_manager_and_lockfile_mismatch() {
        let directory =
            std::env::temp_dir().join(format!("loadout-lock-test-{}", std::process::id()));
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
            found.iter().any(
                |r| r.name == "postgres" && r.constraint.as_deref() == Some("db.internal:5432")
            )
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
}
