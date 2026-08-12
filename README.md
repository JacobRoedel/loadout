# Loadout

[![CI](https://github.com/JacobRoedel/loadout/actions/workflows/ci.yml/badge.svg)](https://github.com/JacobRoedel/loadout/actions/workflows/ci.yml)

Loadout is a local, read-only development-environment readiness checker. Run it from a repository to learn whether the runtimes, package managers, environment variables, and basic dependency state needed for development are present.

It discovers requirements from metadata your repository already has. Loadout does not require or create a Loadout YAML/configuration file, install software, execute package-manager install commands, or make network requests while checking a repository, unless you explicitly opt in with `--services` (see [Service connectivity checks](#service-connectivity-checks)).

## Install and run

Install a prebuilt binary (Linux and macOS; downloads from [GitHub Releases](https://github.com/JacobRoedel/loadout/releases)):

```sh
curl -fsSL https://raw.githubusercontent.com/JacobRoedel/loadout/main/install.sh | sh
```

This installs to `~/.local/bin` by default. Override with `LOADOUT_VERSION=v0.2.0` or `LOADOUT_INSTALL_DIR=$HOME/bin` environment variables. On Windows, or to build from source, install with Cargo instead:

```sh
cargo install --path .
```

Then run a check from the repository root, or provide a path:

```sh
loadout check
loadout check /path/to/repository
```

Build without installing:

```sh
cargo build --release
./target/release/loadout check /path/to/repository
```

## What a check does

Loadout walks the repository while skipping generated/dependency directories such as `.git`, `node_modules`, `target`, virtual environments, `dist`, `build`, and CDK output. For every discovered requirement it:

1. Reads the declared constraint, where one exists.
2. Locates the required command on `PATH` and asks it for its version.
3. Checks documented environment-variable names without printing their values.
4. Reports pass, failure, or warning results with the source metadata path.

Failures mean a required command, version, or environment variable is missing. Warnings are advisory—for example, dependencies that do not appear to be installed. Missing supported commands include a non-executing installation hint for macOS, Linux, or Windows when Loadout knows one.

### Supported repository signals

| Ecosystem | Metadata Loadout reads | Checks it adds |
| --- | --- | --- |
| Node.js | `package.json` engines, `packageManager`, Volta, `.nvmrc`, `.node-version`, npm/pnpm/Yarn/Bun lockfiles | Node, declared package manager, dependency-install warning |
| Rust | `Cargo.toml`, `rust-toolchain`, `rust-toolchain.toml` | `rustc`, `cargo`, build-artifact warning |
| Python | `pyproject.toml`, `.python-version`, requirements files, uv/Poetry/Pipenv lockfiles | Python, applicable package tool, virtualenv warning with a tool-matched install command (`uv sync`, `poetry install`, `pipenv install`, `pip install -r <file>`, or `pip install -e .`) |
| Go | `go.mod` | Go and its declared minimum version |
| Java | Maven `pom.xml`, Gradle files/wrapper | Java |
| Ruby | `.ruby-version`, `Gemfile`, `Gemfile.lock` | Ruby, Bundler, and a `bundle install` warning when `.bundle/config` declares a local vendor path that's missing |
| .NET | `global.json`, `*.sln`, `*.csproj`, `*.fsproj` | .NET SDK, including the SDK version from `global.json` |
| PHP | `composer.json`, `composer.lock` | PHP and Composer |
| Elixir | `mix.exs` | Elixir and Mix |
| Dart / Flutter | `pubspec.yaml` | Dart SDK constraint; Flutter when a Flutter section is declared |
| JVM | `pom.xml`, `build.gradle`, `build.gradle.kts`, `gradlew` | Java, including common Maven/Gradle Java-version declarations |
| Universal version managers | asdf `.tool-versions`, mise `mise.toml`/`.mise.toml` | Version constraints for whichever pinned tools Loadout recognizes (Node, Python, Ruby, Go, Rust, Java, Terraform, Yarn, pnpm, Bun) |
| Infrastructure | `Dockerfile`, Compose files, Terraform `.tf` files | Docker, Terraform |
| Data tooling | `postgresql.conf`, `.psqlrc`, `redis.conf`, `.rediscli_history` | `psql`, `redis-cli` |
| Environment | Root `.env*.example`, `.env*.sample`, `.env*.template` | Non-empty environment variables, required unless marked optional |
| Git hooks | `.pre-commit-config.yaml`/`.yml` | `pre-commit`, and a warning to run `pre-commit install` when `.git/hooks/pre-commit` is missing |
| Environment tooling | `devcontainer.json`, `flake.nix`, `Brewfile` | Docker + Dev Container CLI, Nix, or Homebrew |
| CI runtime pins | `.github/workflows/*.yml`/`.yaml` | Node, Python, Ruby, and Java versions declared in setup-action inputs |

### Environment file awareness

Loadout recognizes every root-level file matching `.env*.example`, `.env*.sample`, or `.env*.template`—not just `.env.example`, but framework and platform variants like `.env.development.example`, `.env.test.example`, or `.env.local.example` too.

Variables are satisfied by a non-empty shell value or a value from any root `.env`, `.env.local`, `.env.development[.local]`, `.env.test[.local]`, `.env.production[.local]`, or [direnv](https://direnv.net/)'s `.envrc` file. Values are never printed or included in JSON output. A root-level `.envrc` also adds a `direnv` command requirement. Loadout does not evaluate shell expressions—`.envrc` is scanned for simple `KEY=VALUE`/`export KEY=VALUE` lines the same way `.env` files are, so constructs beyond plain assignments (`use flake`, conditionals, `layout python`) are ignored.

A variable is treated as **optional** (a WARN instead of a FAIL when unset) only when the example file explicitly says so—Loadout never infers it from a placeholder value being present, since `API_KEY=your-key-here` is a common convention for required variables too. Mark a variable optional with a comment containing the word "optional," either on the line above it or trailing the line itself:

```sh
# Optional: defaults to 3000 if not set
PORT=

API_KEY=your-key-here # optional
```

### Workspaces and monorepos

Loadout recognizes npm/Yarn/Bun workspaces (`package.json` `workspaces`), pnpm workspaces (`pnpm-workspace.yaml`), Cargo workspaces (`Cargo.toml` `[workspace]`), uv workspaces (`pyproject.toml` `[tool.uv.workspace]`), and Turborepo (`turbo.json`). When a workspace root is detected, the dependency-install and build-artifact checks run once at the true root instead of once per member package, and the recommended install command reflects the workspace's package manager (for example `pnpm install --frozen-lockfile` run from the workspace root).

### Universal version managers (asdf, mise)

If a repository uses asdf (`.tool-versions`) or mise (`mise.toml`/`.mise.toml`) instead of per-language version files, Loadout reads the pinned versions for the plugins it recognizes and checks them the same way as `.nvmrc`/`.python-version`/etc. A `rust`/`rustc` entry checks both `rustc` and `cargo`. Entries pinned to `system` or `latest` are skipped—there's no specific version to check against. Plugins Loadout doesn't recognize are silently ignored rather than guessed at.

### Service connectivity checks

Pass `--services` to `check` or `doctor` to additionally verify that configured runtime dependencies are reachable. This is the one Loadout mode that makes network or local-socket connections, so it is opt-in and never runs by default:

```sh
loadout check --services
loadout doctor --services
```

- **Database and cache URLs** — any environment variable (shell or `.env`/`.env.local`) whose value starts with `postgres://`, `postgresql://`, `mysql://`, `redis://`, `rediss://`, `mongodb://`, or `amqp://` is dialed with a 2-second TCP connection attempt to its host and port.
- **Docker daemon** — checked with `docker info` when the repository already has Docker signals (a `Dockerfile` or Compose file).
- **AWS identity** — checked with `aws sts get-caller-identity` when the `aws` CLI is on `PATH` and credentials are configured (`AWS_PROFILE`, `AWS_ACCESS_KEY_ID`, or `~/.aws/credentials`/`~/.aws/config`).

Every connectivity check only ever reports pass/warn status plus the host and port it dialed. Credentials, full connection strings, and command output are never printed or included in JSON output.

## Add one-off checks

Use repeatable `--require` flags for requirements that are not represented in repository metadata:

```sh
loadout check \
  --require 'cmd:docker@>=26' \
  --require cmd:terraform \
  --require env:DATABASE_URL
```

Supported forms are:

- `cmd:NAME` — require an executable on `PATH`.
- `cmd:NAME@CONSTRAINT` — require an executable and matching version.
- `env:NAME` — require a non-empty environment variable.

Loadout invokes a command directly with `--version`; it does not evaluate shell expressions or run arbitrary probes.

## Profiles for local development and CI

Profiles are optional reusable groups of checks. They are invocation-only, so they do not add repository configuration.

| Profile | Requirements |
| --- | --- |
| `web` | `node`, `npm` |
| `rust` | `rustc`, `cargo` |
| `python` | `python` (accepts `python3` fallback) |
| `containers` | `docker` |
| `infra` | `terraform` |
| `data` | `psql`, `redis-cli` |

Use one or more profiles:

```sh
loadout check --profile web --profile containers
loadout check --profile web,infra
```

For CI, provide comma-separated profiles with an environment variable:

```sh
LOADOUT_PROFILE=web,containers loadout check --strict
```

## Reporting, filtering, and exit codes

Human-readable output is the default. Use `--json` for automation; it includes a stable `schema_version` (`loadout/v1`), the repository path, counts, and result objects with status, kind, source, constraint, detected version, and message.

```sh
loadout check --json
loadout check --sarif > loadout.sarif
loadout check --summary
loadout check --explain node
loadout check --only environment
loadout check --only node
loadout check --skip dependency_state
loadout check --quiet
loadout check --strict
```

- `--only <kind|name>` includes checks that match `command`, `environment`, `dependency_state`, `connectivity`, or an exact check name. Repeat it to include several filters.
- `--skip <kind|name>` excludes matching checks. Repeat it as needed.
- `--quiet` hides passing results in human output and prints nothing when every selected check passes.
- `--summary` prints only final counts; `--explain <check>` shows the metadata source and interpreted version constraint for a named check.
- `--sarif` emits a SARIF 2.1.0 report containing failures and warnings for CI/code-scanning systems.
- `--no-color` disables ANSI color codes.
- `--strict` treats warnings as failures for this invocation.

Exit codes:

| Code | Meaning |
| --- | --- |
| `0` | All selected required checks passed; warnings are allowed unless `--strict` is used. |
| `1` | At least one required check failed, or a warning occurred under `--strict`. |
| `2` | Invalid CLI input or an unreadable target path. |

## CI integration

### `--changed` for pull request checks

Pass `--changed <BASE_REF>` to only evaluate requirements whose source file changed relative to a git ref (for example the PR's base branch). Requirements that cannot be attributed to a specific file—profiles, `--require` flags, Docker, AWS—are always included:

```sh
loadout check --changed origin/main
```

This requires enough git history to diff against `BASE_REF`; in GitHub Actions, use `actions/checkout` with `fetch-depth: 0` or a depth that includes the base branch.

### `--annotate` for inline PR annotations

Pass `--annotate` to additionally emit GitHub Actions workflow-command annotations (`::error`/`::warning`) for every failing or warning check, pointing at the exact source file:

```sh
loadout check --annotate
```

### GitHub Action

Use the reusable composite action to run Loadout in any workflow without installing it yourself:

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- uses: JacobRoedel/loadout@main
  with:
    profile: web,containers
    strict: 'true'
    changed: origin/${{ github.base_ref }}
```

Inputs: `path` (default `.`), `profile`, `strict`, `services`, `changed`, and `version` (defaults to `main`). The action runs `check --annotate`, so the step fails naturally when a required check fails and annotations appear inline on the pull request.

Pin `version` to a release tag (e.g. `version: v0.2.0`) to skip the from-source build entirely — on Linux and macOS runners, the action downloads the matching prebuilt binary instead, the same one `install.sh` uses. Windows runners, and any `version` that isn't a release tag (a branch, a commit, `main`), always build from source.

## Ignoring intentional exceptions

Add a `.loadoutignore` file at the repository root to permanently skip specific checks—one pattern per line, blank lines and `#` comments ignored:

```
# We don't run Postgres locally; CI provisions it separately
DATABASE_URL

# Skip all connectivity checks on this machine
connectivity

# Ignore a check only for one workspace member (name@source substring match)
node_modules@packages/legacy-app
```

Each line matches the same way `--only`/`--skip` do: a check `kind` (`command`, `environment`, `dependency_state`, `connectivity`), an exact check name, or `name@source` to scope the exception to sources containing that substring. `.loadoutignore` is read by both `check` and `doctor`; pass `--no-ignore-file` to bypass it for one invocation (useful in CI when you want the unfiltered picture). Skipped checks are counted and reported, never silently dropped. This is intentionally a short exception list, not a place to reconfigure how Loadout behaves.

## Advisory mode

`loadout init` explains which requirements the current repository already exposes. It is a discovery aid, not a setup command:

```sh
loadout init
loadout init --json
```

It writes no files and does not make its output a required input to Loadout.

`loadout init` also lists the runnable commands it finds in the repository, so you can see what's available for dev/test/build without opening every file:

- **npm scripts** — `package.json` `scripts`
- **Makefile** — top-level targets (`make <target>`)
- **Justfile** — top-level recipes (`just <recipe>`)
- **Taskfile** — `Taskfile.yml`/`Taskfile.yaml` tasks (`task <name>`)
- **Docker Compose** — service names (`docker compose up <service>`)

This is advisory only—commands are listed, never executed.

## Guided remediation with `loadout doctor`

`loadout doctor` runs the same checks as `loadout check`, then groups every blocker into ordered steps instead of a flat list:

```sh
loadout doctor
loadout doctor --json
loadout doctor --open-docs
```

1. **Install missing tools and fix version mismatches** — nothing else can succeed until these are resolved.
2. **Install project dependencies** — npm/pnpm/yarn/bun/cargo/pip installs, using the same install-command recommendation as `check`.
3. **Configure environment variables** — usually the last thing you set before running the app.

Each blocker lists a documentation link for the missing tool. Pass `--open-docs` to open each unique link in your default browser instead of just printing it. Like every other Loadout command, `doctor` never installs anything or writes files—it only reports and links out.

## Shell completions and man page

Generate integrations to stdout and place them where your shell expects them:

```sh
loadout completions zsh > ~/.zfunc/_loadout
loadout completions bash > ~/.local/share/bash-completion/completions/loadout
loadout completions fish > ~/.config/fish/completions/loadout.fish
loadout completions powershell > loadout.ps1
loadout completions elvish > loadout.elv
loadout man > loadout.1
```

## Development

```sh
cargo fmt --check
cargo test --locked
cargo clippy -- -D warnings
cargo build --release --locked
```

See [AGENTS.md](AGENTS.md) for the branch and pull-request workflow used for feature and bug-fix changes.

### Releasing

Pushing a tag matching the package version (for example, `v0.2.0` for Cargo package version `0.2.0`) triggers `.github/workflows/release.yml`, which builds `loadout` for Linux (x86_64, aarch64), macOS (x86_64, aarch64), and Windows (x86_64), then publishes a GitHub Release with each archive and a `checksums.txt`. `install.sh` and `action.yml` both consume these release assets. `workflow_dispatch` can be used to test the build matrix without publishing (only tag pushes publish a release).

These predictable, checksummed archives are also what a future Homebrew formula or winget manifest would point at, though publishing to those package managers requires a separate submission and isn't part of this repository.
