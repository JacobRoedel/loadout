# Loadout

Loadout is a local, read-only, single-binary Rust CLI (`src/`) that checks whether a repository's development environment is ready: are the right runtimes and package managers installed, do their versions satisfy what the repo declares, are dependencies actually installed, and are required environment variables set. It never installs software, writes configuration files, or executes package-manager commands on your behalf — it only reads metadata the repository already has (`package.json`, `Cargo.toml`, `pyproject.toml`, lockfiles, `.env.example` variants, etc.) and reports pass/fail/warn results. The one deliberate exception is `--services`, an opt-in flag that makes network/local-socket connections to verify configured runtime dependencies (database URLs, Docker daemon, AWS identity) are reachable — every other code path is fully offline.

## Commands

- `loadout check [PATH]` — the core command. Discovers requirements, evaluates them against the local machine, and reports pass/fail/warn. Supports `--require` (one-off `cmd:`/`env:` checks), `--profile` (bundled requirement groups), `--only`/`--skip` (filter by kind or name), `--strict` (warnings become failures), `--services` (opt-in connectivity checks), `--changed <BASE_REF>` (scope to requirements whose source file changed vs. a git ref, for fast PR checks), `--annotate` (GitHub Actions `::error`/`::warning` workflow commands), `--json`, `--quiet`, `--no-color`.
- `loadout doctor [PATH]` — the same checks as `check`, but blockers are grouped into ordered remediation steps (tools → dependency installs → environment variables) with documentation links, optionally opened in a browser via `--open-docs`.
- `loadout init [PATH]` — a discovery aid, not a setup command. Lists the requirements Loadout already detects and the runnable commands it finds (npm scripts, Makefile targets, Justfile recipes, Taskfile tasks, Docker Compose services). Writes no files.
- `loadout completions <shell>` / `loadout man` — generate shell completions or a man page to stdout.
- `action.yml` — a reusable composite GitHub Action so other repositories can run Loadout in CI via `uses: JacobRoedel/loadout@main` without installing Rust themselves.

## Core architecture (`src/`)

`main.rs` only parses the CLI and dispatches to a subcommand runner; each concern below lives in its own module (all `pub(crate)`, no `lib.rs` — this is a single binary crate):

- **`model.rs`** — the shared data types (`Requirement`, `ResultItem`, `Kind`, `Status`, `Report`, `CheckOptions`/`DoctorOptions`, etc.) and their small constructors (`command`, `warn`, `display`). Every other module depends on this one.
- **`cli.rs`** — the `clap` `Cli`/`Commands`/`Shell`/`Profile` definitions only.
- **`discover.rs`** walks the repository (skipping `.git`, `node_modules`, `target`, venvs, build output, etc.) and returns a `Vec<Requirement>` built from whatever metadata it finds — engine/version constraints, package managers, workspace roots (npm/pnpm/Yarn/Bun/Cargo/uv workspaces, Turborepo), dependency-install state. Per-ecosystem detectors (Node, Rust, Python, Go, Ruby, Java, asdf/mise, pre-commit) live here too.
- **`dotenv.rs`** — `.env*.example`/`.sample`/`.template` discovery and parsing, plus reading local `.env*`/`.envrc` values.
- **`connectivity.rs`** — the opt-in `--services` checks (database/cache URL reachability, Docker daemon, AWS identity).
- **`evaluate.rs`** — `evaluate` dispatches by `Kind` (`Command`, `Environment`, `DependencyState`, `Connectivity`) to run the actual check: locating a binary on `PATH` and comparing its `--version` output against a `semver` constraint (delegating `Connectivity` to `connectivity.rs`), checking an environment variable is non-empty, etc. Version normalization/matching and `--require` parsing live here too.
- **`scripts.rs`** — the `init`-only, non-validating scan for npm scripts, Makefile/Justfile/Taskfile targets, and Compose services.
- **`ignore_file.rs`** — `.loadoutignore` parsing and filtering, shared by `check` and `doctor`.
- **`check.rs`** — **`gather_results`** expands `--profile`/`--require` inputs on top of `discover`'s output, optionally adds `--services` connectivity requirements, and evaluates every `Requirement` into a `ResultItem`. Shared by `check` and `doctor` so both commands see identical results. Also owns `run_check` and the human/annotation output for `check`.
- **`doctor.rs`** / **`init.rs`** — the `doctor` and `init` subcommand runners.
- **`tests.rs`** — the crate's unit tests, kept in one module (rather than split per-file) since many tests exercise several modules together.

Filtering layers apply in this order: `.loadoutignore` (persistent repo-local exceptions) → `--only`/`--skip` (invocation-scoped) → `--changed` (git-diff scoped).

## Verification commands

```sh
cargo fmt --check
cargo test --locked
cargo clippy -- -D warnings
cargo build --release --locked
```

CI (`.github/workflows/ci.yml`) runs all of these on every PR and push to `main`. Note: the CI runner's Rust toolchain can be newer than a local install, so clippy lints that pass locally can still fail in CI — treat CI as the source of truth and re-check after any lint-sensitive change.

# Contributor workflow

- Before making any feature or bug-fix change, create a dedicated branch from the current `main` branch.
- Use `feat/<short-description>` for features, `fix/<short-description>` for bug fixes, and `docs/<short-description>` for documentation-only changes.
- Never make feature or bug-fix commits directly on `main`.
- Keep each branch focused on one change, run the relevant verification commands, push the branch, and open a pull request before merging.
