# Loadout

[![CI](https://github.com/JacobRoedel/loadout/actions/workflows/ci.yml/badge.svg)](https://github.com/JacobRoedel/loadout/actions/workflows/ci.yml)

**Know whether a repository is ready to run before you start working.**

Loadout is a fast, local, read-only Rust CLI that discovers the tools, versions, dependencies, and environment variables a repository expects, then reports what is ready and what needs attention. It learns from metadata already in the project—there is no Loadout configuration file to maintain.

It never installs software, edits project files, executes package-manager commands, or makes network requests during a normal check. Service connectivity is an explicit `--services` opt-in.

[Product walkthrough](docs/slides.html) · [Releases](https://github.com/JacobRoedel/loadout/releases) · [Security policy](SECURITY.md)

## Quick start

Install the latest checksummed release on macOS or Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/JacobRoedel/loadout/main/install.sh | sh
```

Then inspect the current repository:

```sh
loadout check
```

Or point it at another project:

```sh
loadout check /path/to/repository
```

The installer uses `~/.local/bin` by default. Pin a release with `LOADOUT_VERSION=v0.2.0`, or choose a directory with `LOADOUT_INSTALL_DIR=/your/bin`. To build locally instead:

```sh
cargo build --release
./target/release/loadout check /path/to/repository
```

## What you get

`check` is the factual report. `doctor` turns its blockers into an ordered remediation plan. `init` shows the requirements and runnable project commands Loadout discovered, without changing anything.

```text
$ loadout check
PASS  node (>=20) [22.20.0]
WARN  node_modules     run npm ci
FAIL  DATABASE_URL     set in local .env

1 passed, 1 failed, 1 warning

$ loadout doctor
1. Install project dependencies
   npm ci
2. Configure environment variables
   DATABASE_URL · .env.example
```

Failures are blockers. Warnings are guidance unless you use `--strict`.

## What Loadout discovers

| Area | Examples of repository signals |
| --- | --- |
| Runtimes and package managers | `package.json`, `Cargo.toml`, `pyproject.toml`, `go.mod`, `pom.xml`, `Gemfile`, `composer.json`, `mix.exs`, `pubspec.yaml` |
| Version pins | `engines`, `packageManager`, Volta, `.nvmrc`, `rust-toolchain`, `.python-version`, `global.json`, asdf and mise files |
| Dependency state | Node lockfiles, Rust build output, Python environments and lockfiles, local Ruby bundle configuration |
| Environment | Root `.env*.example`, `.sample`, and `.template` files; local `.env*` and simple `.envrc` assignments satisfy checks without exposing values |
| Tooling and CI | Docker/Compose, Terraform, dev containers, Nix, Brewfiles, pre-commit, and runtime pins in GitHub Actions |
| Workspaces | npm, pnpm, Yarn, Bun, Cargo, uv, and Turborepo workspaces are evaluated at their real root |

Repository walking skips generated and dependency directories such as `.git`, `node_modules`, `target`, virtual environments, `dist`, and `build`.

## Core commands

```sh
# Structured output for scripts and agents
loadout check --json

# Explain the source and interpretation of a check
loadout check --explain node

# Make warnings fail a CI run
loadout check --strict

# Add an explicit requirement not represented in metadata
loadout check --require 'cmd:docker@>=26' --require env:DATABASE_URL

# Reuse a built-in requirement group
loadout check --profile web,containers

# Show an ordered fix plan; never applies it
loadout doctor

# Limit checks to metadata changed from a base ref
loadout check --changed origin/main
```

`--require` accepts `cmd:NAME`, `cmd:NAME@CONSTRAINT`, and `env:NAME`. Built-in profiles are `web`, `rust`, `python`, `containers`, `infra`, and `data`.

Use `loadout check --help` or `loadout doctor --help` for the complete option reference, including filtering, SARIF, quiet/summary modes, ignore-file controls, and shell completion generation.

## Agent- and automation-friendly

JSON output has a stable `loadout/v1` schema with result status, kind, name, source, declared constraint, detected version, and message. Secrets are never included: environment variables are reported only as set or missing, and connection credentials are not printed.

```sh
loadout check --json
```

Loadout also works as a GitHub Action. Pin both the action and binary version for a reproducible CI check:

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- uses: JacobRoedel/loadout@v0.2.0
  with:
    version: v0.2.0
    profile: web,containers
    strict: 'true'
    changed: origin/${{ github.base_ref }}
```

On Linux and macOS, a pinned release downloads the matching prebuilt archive and verifies its SHA-256 checksum. Non-release refs and Windows runners build from source. The Action emits GitHub annotations for findings.

## Optional service checks

Pass `--services` when you want to test whether configured runtime dependencies are reachable:

```sh
loadout check --services
loadout doctor --services
```

This is the only network or local-socket behavior. It can test recognized database/cache URLs with a 2-second TCP attempt, the Docker daemon for Docker-enabled projects, and AWS identity when the AWS CLI is configured. It reports status and host/port only—never credentials or complete connection strings.

## Intentional exceptions

Use a root `.loadoutignore` file for a small, explicit list of checks your project deliberately does not need:

```text
# CI provides this service
DATABASE_URL

# Skip all service connectivity checks locally
connectivity

# Limit an exception to one workspace member
node_modules@packages/legacy-app
```

Patterns may be a check kind, exact name, or `name@source-substring`. Skipped findings remain counted and visible.

## Development and releases

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked -- -D warnings
cargo build --release --locked
```

Feature and bug-fix changes use focused branches and pull requests; see [AGENTS.md](AGENTS.md). Pushing a tag that matches the package version, such as `v0.2.1`, creates the public release: five platform binaries, SHA-256 checksums, an SPDX SBOM, and GitHub build provenance. See [SECURITY.md](SECURITY.md) to report a vulnerability privately.
