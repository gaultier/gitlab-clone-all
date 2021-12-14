use anyhow::{bail, Context, Result};
use bytesize::ByteSize;
use clap::Parser;
use console::style;
use git2::ErrorCode;
use git2::{Cred, RemoteCallbacks};
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use reqwest::Client;
use serde::Deserialize;
use std::cell::RefCell;
use std::str::FromStr;
use std::sync::Arc;
use std::{path::PathBuf, time::Duration};
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug)]
enum ProjectAction {
    ProjectToClone,
    ProjectCloned {
        project_path: String,
        received_bytes: usize,
    },
}

#[derive(Debug, PartialEq)]
enum CloneMethod {
    Ssh,
    Https,
}

impl FromStr for CloneMethod {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "https" => Ok(CloneMethod::Https),
            "ssh" => Ok(CloneMethod::Ssh),
            _ => Err("no match"),
        }
    }
}

/// Clone all git repositories from gitlab
#[derive(Parser)]
struct Opts {
    /// Root directory where to clone all the projects
    #[clap(short, long, default_value = ".")]
    directory: PathBuf,
    #[clap(short, long, default_value = "")]
    api_token: String,
    #[clap(short, long, possible_values = &["https","ssh"], default_value="https")]
    clone_method: CloneMethod,
}

#[derive(Debug, Deserialize)]
struct Group {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct Project {
    id: u64,
    ssh_url_to_repo: String,
    http_url_to_repo: String,
    path_with_namespace: String,
}

async fn fetch_groups(client: &reqwest::Client) -> Result<Vec<Group>> {
    let req = client
        .get("https://gitlab.ppro.com/api/v4/groups?statistics=false&top_level=&with_custom_attributes=false&all_available=true&top_level&order_by=id&sort=asc&pagination=keyset&per_page=100"); // TODO: pagination

    let json = req.send().await?.text().await?;

    let groups: Vec<Group> = serde_json::from_str(&json)
        .with_context(|| format!("Failed to parse to JSON: json={}", json))?;

    Ok(groups)
}

async fn fetch_group_projects_paginated(
    client: reqwest::Client,
    group_id: u64,
    project_id_after: Option<u64>,
) -> Result<Vec<Project>> {
    let  req = client
        .get(format!("https://gitlab.ppro.com/api/v4/groups/{}/projects?statistics=false&top_level=&with_custom_attributes=false&all_available=true&top_level&order_by=id&sort=asc&pagination=keyset&per_page=100&id_after={}", group_id, project_id_after.unwrap_or(0)));

    let json = req.send().await?.text().await?;

    let projects: Vec<Project> = serde_json::from_str(&json)
        .with_context(|| format!("Failed to parse to JSON: json={}", json))?;

    Ok(projects)
}

fn make_http_client(api_token: &str) -> Result<Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "PRIVATE-TOKEN",
        HeaderValue::from_str(api_token)
            .with_context(|| "Invalid token: cannot be set as HTTP header")?,
    );

    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .default_headers(headers)
        .build()
        .with_context(|| "Failed to create http client")
}

async fn clone_projects(
    mut rx_projects: Receiver<Project>,
    opts: Arc<Opts>,
    tx_projects_actions: Sender<ProjectAction>,
) -> Result<()> {
    match std::fs::create_dir(&opts.directory) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => {
            bail!(
                "Failed to create the destination directory: directory={} err={}",
                opts.directory.as_path().to_string_lossy(),
                err
            );
        }
    }

    while let Some(project) = rx_projects.recv().await {
        let path = opts.directory.join(&project.path_with_namespace);
        log::trace!("Cloning project {:?} fs_path={:?}", &project, &path);

        let opts = opts.clone();
        let tx_projects_actions = tx_projects_actions.clone();
        tokio::spawn(async move {
            let received_bytes = RefCell::new(0usize);
            let mut builder = if opts.clone_method == CloneMethod::Ssh {
                let mut callbacks = RemoteCallbacks::new();
                callbacks.credentials(|_url, username_from_url, _allowed_types| {
                    Cred::ssh_key(
                        username_from_url.unwrap(),
                        None,
                        std::path::Path::new(&format!(
                            "{}/.ssh/id_rsa_gitlab",
                            std::env::var("HOME").unwrap()
                        )),
                        None,
                    )
                });
                callbacks.transfer_progress(|stats| {
                    received_bytes.replace_with(|old| *old + stats.received_bytes());
                    true
                });
                // Prepare fetch options.
                let mut fo = git2::FetchOptions::new();
                fo.remote_callbacks(callbacks);

                // Prepare builder.
                let mut builder = git2::build::RepoBuilder::new();
                builder.fetch_options(fo);
                builder
            } else {
                git2::build::RepoBuilder::new()
            };

            let url_to_repo = match opts.clone_method {
                CloneMethod::Ssh => &project.ssh_url_to_repo,
                CloneMethod::Https => &project.http_url_to_repo,
            };

            match builder.clone(url_to_repo, &path) {
                Ok(_repo) => {
                    log::info!("Cloned project={:?}", &project);
                }
                // Swallow this error
                // TODO: Should we pull in that case?
                Err(e) if e.code() == ErrorCode::Exists => {}
                Err(e) => log::error!("Failed to clone: project={:?} err={}", &project, e),
            };
            tx_projects_actions
                .try_send(ProjectAction::ProjectCloned {
                    project_path: project.path_with_namespace,
                    received_bytes: received_bytes.take(),
                })
                .with_context(|| "Failed to send ProjectCloned")
                .unwrap();
        });
    }

    log::debug!("Finished cloning");
    Ok(())
}

async fn fetch_all_projects_for_group(
    client: reqwest::Client,
    tx_projects: Sender<Project>,
    tx_projects_actions: Sender<ProjectAction>,
    group: Group,
) {
    let mut project_id_after = None;
    loop {
        let res = fetch_group_projects_paginated(client.clone(), group.id, project_id_after).await;

        match res {
            Err(err) => {
                log::error!(
                    "Failed to fetch projects: group_id={} err={}",
                    group.id,
                    err
                );
                break;
            }
            Ok(projects) => {
                let new_project_id_after = projects.iter().map(|p| p.id).last();
                for project in projects {
                    log::debug!("group_id={} project={:?}", group.id, project);
                    tx_projects_actions
                        .send(ProjectAction::ProjectToClone)
                        .await
                        .with_context(|| "Failed to send ProjectToClone")
                        .unwrap();
                    tx_projects
                        .send(project)
                        .await
                        .with_context(|| "Failed to send project")
                        .unwrap();
                }

                if new_project_id_after == project_id_after || new_project_id_after.is_none() {
                    break;
                }
                project_id_after = new_project_id_after;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let opts: Arc<Opts> = Arc::new(Opts::parse());

    let (tx_projects, rx_projects) = tokio::sync::mpsc::channel::<Project>(500);
    let (tx_projects_actions, mut rx_projects_actions) =
        tokio::sync::mpsc::channel::<ProjectAction>(500);

    let opts_1 = opts.clone();
    let tx_projects_actions_1 = tx_projects_actions.clone();
    tokio::spawn(async move {
        if let Err(err) = clone_projects(rx_projects, opts_1, tx_projects_actions_1).await {
            log::error!("Failed to clone projects: err={}", err);
        }
    });

    let client = make_http_client(&opts.api_token)?;
    let groups = fetch_groups(&client).await?;
    log::debug!("Groups: {:?}", groups);

    for group in groups {
        let client = client.clone();
        let tx_projects_2 = tx_projects.clone();

        let tx_projects_actions_2 = tx_projects_actions.clone();
        tokio::spawn(async move {
            fetch_all_projects_for_group(client, tx_projects_2, tx_projects_actions_2, group).await;
        });
    }

    let mut todo_count: Option<usize> = None;
    let mut total_count = 0usize;
    let mut cloned_count = 0usize;
    let mut total_bytes = 0usize;

    loop {
        let group_message = rx_projects_actions.recv().await;
        log::debug!("todo_count={:?} message={:?}", todo_count, group_message);

        match group_message {
            None => {
                unreachable!();
            }
            Some(ProjectAction::ProjectToClone) => {
                todo_count = todo_count.map(|n| n + 1).or(Some(1));
                total_count += 1;
            }
            Some(ProjectAction::ProjectCloned {
                project_path,
                received_bytes,
            }) => {
                cloned_count += 1;
                total_bytes += received_bytes;
                todo_count = todo_count.map(|n| n - 1);
                println!(
                    "{} {} ({})",
                    style("✓").green(),
                    project_path,
                    ByteSize(received_bytes as u64)
                );

                if todo_count == Some(0) {
                    log::debug!("Done");
                    println!(
                        "Successfully cloned: {}/{} ({})",
                        cloned_count,
                        total_count,
                        ByteSize(total_bytes as u64)
                    );
                    return Ok(());
                }
            }
        };
    }
}
