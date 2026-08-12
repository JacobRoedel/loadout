mod check;
mod cli;
mod connectivity;
mod discover;
mod doctor;
mod dotenv;
mod evaluate;
mod ignore_file;
mod init;
mod model;
mod scripts;

#[cfg(test)]
mod tests;

use std::{env, io, path::PathBuf};

use clap::{CommandFactory, Parser};
use clap_complete::{generate, shells};
use clap_mangen::Man;

use check::run_check;
use cli::{Cli, Commands, Shell};
use doctor::run_doctor;
use init::run_init;
use model::{CheckOptions, DoctorOptions};

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Check {
            path,
            requirements,
            profile,
            json,
            sarif,
            summary,
            explain,
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
                sarif,
                summary,
                explain,
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
