use std::{
    collections::HashMap,
    env,
    net::{TcpStream, ToSocketAddrs},
    path::PathBuf,
    process::Command,
    time::Duration,
};

use crate::model::{Kind, Requirement, ResultItem, Status};

/// Builds opt-in connectivity checks from already-discovered requirements and
/// the resolved environment: reachable database/cache URLs, a running Docker
/// daemon (if Docker is used by the repository), and AWS identity (if the
/// AWS CLI is configured). Only invoked when the caller explicitly asks for
/// network connectivity checks.
pub(crate) fn connectivity_requirements(
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
pub(crate) fn parse_service_url(value: &str) -> Option<(&'static str, String, u16)> {
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

pub(crate) fn aws_configured() -> bool {
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

pub(crate) fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Verifies a configured service is reachable. Only ever reports pass/warn
/// status and, for network targets, the host:port that was dialed—never a
/// credential, connection string, or command output.
pub(crate) fn evaluate_connectivity(requirement: &Requirement) -> ResultItem {
    match requirement.name.as_str() {
        "docker" => evaluate_docker_daemon(requirement),
        "aws" => evaluate_aws_identity(requirement),
        service => evaluate_tcp_reachable(service, requirement),
    }
}

pub(crate) fn evaluate_tcp_reachable(service: &str, requirement: &Requirement) -> ResultItem {
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

pub(crate) fn evaluate_docker_daemon(requirement: &Requirement) -> ResultItem {
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

pub(crate) fn evaluate_aws_identity(requirement: &Requirement) -> ResultItem {
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
