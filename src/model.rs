use std::path::Path;

use serde::Serialize;

use crate::cli::Profile;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Command,
    Environment,
    DependencyState,
    Connectivity,
}

#[derive(Clone, Debug)]
pub(crate) struct Requirement {
    pub(crate) kind: Kind,
    pub(crate) name: String,
    pub(crate) constraint: Option<String>,
    pub(crate) source: String,
    pub(crate) required: bool,
    pub(crate) message: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    Pass,
    Fail,
    Warn,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResultItem {
    pub(crate) status: Status,
    pub(crate) kind: Kind,
    pub(crate) name: String,
    pub(crate) constraint: Option<String>,
    pub(crate) source: String,
    pub(crate) found: Option<String>,
    pub(crate) message: String,
}

#[derive(Serialize)]
pub(crate) struct Report {
    pub(crate) path: String,
    pub(crate) results: Vec<ResultItem>,
    pub(crate) passed: usize,
    pub(crate) failed: usize,
    pub(crate) warnings: usize,
    pub(crate) ignored: usize,
}

pub(crate) struct CheckOptions {
    pub(crate) requirements: Vec<String>,
    pub(crate) profiles: Vec<Profile>,
    pub(crate) json: bool,
    pub(crate) no_color: bool,
    pub(crate) only: Vec<String>,
    pub(crate) skip: Vec<String>,
    pub(crate) strict: bool,
    pub(crate) quiet: bool,
    pub(crate) services: bool,
    pub(crate) changed: Option<String>,
    pub(crate) annotate: bool,
    pub(crate) no_ignore_file: bool,
}

pub(crate) struct DoctorOptions {
    pub(crate) requirements: Vec<String>,
    pub(crate) profiles: Vec<Profile>,
    pub(crate) json: bool,
    pub(crate) no_color: bool,
    pub(crate) open_docs: bool,
    pub(crate) services: bool,
    pub(crate) no_ignore_file: bool,
}

#[derive(Serialize)]
pub(crate) struct Advisory {
    pub(crate) path: String,
    pub(crate) writes_files: bool,
    pub(crate) requirements: Vec<AdvisoryItem>,
    pub(crate) scripts: Vec<ScriptGroup>,
}

#[derive(Serialize)]
pub(crate) struct AdvisoryItem {
    pub(crate) requirement: String,
    pub(crate) source: String,
}

#[derive(Serialize)]
pub(crate) struct ScriptGroup {
    pub(crate) tool: &'static str,
    pub(crate) source: String,
    pub(crate) commands: Vec<ScriptCommand>,
}

#[derive(Serialize)]
pub(crate) struct ScriptCommand {
    pub(crate) name: String,
    pub(crate) run: String,
}

pub(crate) fn kind_name(kind: &Kind) -> &str {
    match kind {
        Kind::Command => "command",
        Kind::Environment => "environment",
        Kind::DependencyState => "dependency_state",
        Kind::Connectivity => "connectivity",
    }
}

pub(crate) fn display(path: &Path) -> String {
    path.display().to_string()
}
pub(crate) fn command(
    name: &str,
    constraint: Option<String>,
    source: String,
    required: bool,
) -> Requirement {
    Requirement {
        kind: Kind::Command,
        name: name.into(),
        constraint,
        source,
        required,
        message: None,
    }
}

pub(crate) fn warn(name: &str, source: String, message: &str) -> ResultItem {
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
