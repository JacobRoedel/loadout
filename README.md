# Loadout

[![CI](https://github.com/JacobRoedel/loadout/actions/workflows/ci.yml/badge.svg)](https://github.com/JacobRoedel/loadout/actions/workflows/ci.yml)

Loadout tells you whether your local machine has the prerequisites needed to work in a repository. It reads existing project metadata; it does not add configuration, install tools, run package managers, or use the network.

```sh
cargo install --path .
loadout check
loadout check --require cmd:docker@">=26" --require env:DATABASE_URL
loadout check --json
loadout init
```

It recognizes Node.js (`package.json`, Volta, `.nvmrc`, lockfiles), Rust (`Cargo.toml`, `rust-toolchain`), Python (`pyproject.toml`, `.python-version`, common lockfiles), Go (`go.mod`), Java (`pom.xml`, Gradle), Ruby (`Gemfile`, `.ruby-version`), Docker/Compose, Terraform, and explicit Postgres/Redis development markers. It also reads required variables documented in root `.env.example` or `.env.sample`.

Missing installed dependencies are warnings only. A failed command or environment check exits with status `1`; warnings still exit `0`.

`loadout init` is advisory only: it prints the requirements Loadout detected from existing metadata (or JSON with `--json`) and never creates or requires a Loadout configuration file.

For focused local or CI checks, use `--only environment`, `--skip dependency_state`, or a specific check name such as `--only node`. `--strict` makes warnings exit with status `1`; `--quiet` hides passing checks in human output.

When an executable is missing, Loadout prints a non-executing installation hint tailored to macOS, Linux, or Windows when it knows one. Review the command before running it; Loadout never installs tools itself.

Generate local shell integration without committing generated files:

```sh
loadout completions zsh > ~/.zfunc/_loadout
loadout completions bash > ~/.local/share/bash-completion/completions/loadout
loadout man > loadout.1
```
