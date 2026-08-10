# Loadout

[![CI](https://github.com/JacobRoedel/loadout/actions/workflows/ci.yml/badge.svg)](https://github.com/JacobRoedel/loadout/actions/workflows/ci.yml)

Loadout is a local, read-only development-environment readiness checker. Run it from a repository to learn whether the runtimes, package managers, environment variables, and basic dependency state needed for development are present.

It discovers requirements from metadata your repository already has. Loadout does not require or create a Loadout YAML/configuration file, install software, execute package-manager install commands, or make network requests while checking a repository.

## Install and run

Install from a local checkout:

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
| Python | `pyproject.toml`, `.python-version`, requirements files, uv/Poetry/Pipenv lockfiles | Python, applicable package tool, virtualenv warning |
| Go | `go.mod` | Go and its declared minimum version |
| Java | Maven `pom.xml`, Gradle files/wrapper | Java |
| Ruby | `.ruby-version`, `Gemfile`, `Gemfile.lock` | Ruby, Bundler |
| Infrastructure | `Dockerfile`, Compose files, Terraform `.tf` files | Docker, Terraform |
| Data tooling | `postgresql.conf`, `.psqlrc`, `redis.conf`, `.rediscli_history` | `psql`, `redis-cli` |
| Environment | Root `.env.example` and `.env.sample` | Required non-empty environment variables |

Environment variables are satisfied by a non-empty shell value or a root `.env` / `.env.local` value. Values are never printed or included in JSON output.

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

Human-readable output is the default. Use `--json` for automation; it includes the repository path, counts, and result objects with status, kind, source, constraint, detected version, and message.

```sh
loadout check --json
loadout check --only environment
loadout check --only node
loadout check --skip dependency_state
loadout check --quiet
loadout check --strict
```

- `--only <kind|name>` includes checks that match `command`, `environment`, `dependency_state`, or an exact check name. Repeat it to include several filters.
- `--skip <kind|name>` excludes matching checks. Repeat it as needed.
- `--quiet` hides passing results in human output and prints nothing when every selected check passes.
- `--no-color` disables ANSI color codes.
- `--strict` treats warnings as failures for this invocation.

Exit codes:

| Code | Meaning |
| --- | --- |
| `0` | All selected required checks passed; warnings are allowed unless `--strict` is used. |
| `1` | At least one required check failed, or a warning occurred under `--strict`. |
| `2` | Invalid CLI input or an unreadable target path. |

## Advisory mode

`loadout init` explains which requirements the current repository already exposes. It is a discovery aid, not a setup command:

```sh
loadout init
loadout init --json
```

It writes no files and does not make its output a required input to Loadout.

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
