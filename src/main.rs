use anyhow::{bail, Context, Result};
use bytesize::ByteSize;
use clap::Parser;
use console::style;
use git2::build::CheckoutBuilder;
use git2::{Cred, ErrorCode, RemoteCallbacks};
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use reqwest::Client;
use serde::Deserialize;
use std::cell::RefCell;
use std::str::FromStr;
use std::{path::Component, path::Path, path::PathBuf, time::Duration};
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug)]
enum ProjectAction {
    ToClone,
    Cloned {
        project_path: String,
        received_bytes: usize,
        received_objects: usize,
    },
    Failed {
        project_path: String,
        err: String,
    },
}

#[derive(Debug, PartialEq, Copy, Clone)]
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
    #[clap(short, long, default_value = "gitlab.com")]
    url: String,
    #[clap(short, long, possible_values = &["https","ssh"], default_value="https")]
    clone_method: CloneMethod,
}

#[derive(Debug, Deserialize)]
struct Project {
    id: u64,
    ssh_url_to_repo: String,
    http_url_to_repo: String,
    path_with_namespace: String,
}

async fn fetch_projects_paginated(
    client: reqwest::Client,
    project_id_after: Option<u64>,
    gitlab_url: &str,
) -> Result<Vec<Project>> {
    let  req = client
        .get(format!("https://{}/api/v4/projects?statistics=false&top_level=&with_custom_attributes=false&all_available=true&top_level&order_by=id&sort=asc&pagination=keyset&per_page=100&id_after={}", gitlab_url, project_id_after.unwrap_or(0)));

    let json = req.send().await?.text().await?;

    let projects: Vec<Project> = serde_json::from_str(&json)
        .with_context(|| format!("Failed to parse projects from JSON: json={}", json))?;

    log::debug!("Fetched projects: count={}", projects.len(),);
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
    tx_projects_actions: Sender<ProjectAction>,
    expanded_path: &Path,
    clone_method: CloneMethod,
) -> Result<()> {
    while let Some(project) = rx_projects.recv().await {
        let path = expanded_path.join(&project.path_with_namespace);
        log::debug!("Cloning project {:?} fs_path={:?}", &project, &path);

        let tx_projects_actions = tx_projects_actions.clone();
        tokio::spawn(async move {
            let received_bytes = RefCell::new(0usize);
            let received_objects = RefCell::new(0usize);

            let mut builder = git2::build::RepoBuilder::new();
            let mut callbacks = RemoteCallbacks::new();
            callbacks.transfer_progress(|stats| {
                received_bytes.replace(stats.received_bytes());
                received_objects.replace(stats.received_objects());
                true
            });
            let mut co = CheckoutBuilder::new();
            co.progress(|path, cur, total| {
                log::debug!("{:?} {}/{}", path, cur, total);
            });
            builder.with_checkout(co);
            // Prepare fetch options.
            let mut fo = git2::FetchOptions::new();

            if clone_method == CloneMethod::Ssh {
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
            }
            fo.remote_callbacks(callbacks);
            builder.fetch_options(fo);

            let url_to_repo = match clone_method {
                CloneMethod::Ssh => &project.ssh_url_to_repo,
                CloneMethod::Https => &project.http_url_to_repo,
            };

            match builder.clone(url_to_repo, &path) {
                Ok(_repo) => {
                    log::info!("Cloned project={:?}", &project);
                    tx_projects_actions
                        .try_send(ProjectAction::Cloned {
                            project_path: project.path_with_namespace,
                            received_bytes: received_bytes.take(),
                            received_objects: received_objects.take(),
                        })
                        .with_context(|| "Failed to send ProjectCloned")
                        .unwrap();
                }
                // Swallow this error
                // TODO: Should we pull in that case?
                Err(e) if e.code() == ErrorCode::Exists => {
                    tx_projects_actions
                        .try_send(ProjectAction::Cloned {
                            project_path: project.path_with_namespace,
                            received_bytes: received_bytes.take(),
                            received_objects: received_objects.take(),
                        })
                        .with_context(|| "Failed to send ProjectCloned")
                        .unwrap();
                }
                Err(e) => {
                    log::error!("Failed to clone: project={:?} err={}", &project, e);
                    tx_projects_actions
                        .try_send(ProjectAction::Failed {
                            project_path: project.path_with_namespace,
                            err: e.to_string(),
                        })
                        .with_context(|| "Failed to send ProjectFailed")
                        .unwrap();
                }
            };
        });
    }

    log::debug!("Finished cloning");
    Ok(())
}

async fn fetch_projects(
    client: reqwest::Client,
    tx_projects: Sender<Project>,
    tx_projects_actions: Sender<ProjectAction>,
    gitlab_url: &str,
) -> Result<()> {
    let mut project_id_after = None;
    loop {
        let projects = fetch_projects_paginated(client.clone(), project_id_after, gitlab_url)
            .await
            .with_context(|| "Failed to fetch projects")?;
        log::debug!("Projects: {:?}", &projects);

        let new_project_id_after = projects.iter().map(|p| p.id).last();
        for project in projects {
            log::debug!("project={:?}", &project);
            tx_projects_actions
                .send(ProjectAction::ToClone)
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
    Ok(())
}

// Needed to expand `~` which is otherwise understood literally as a relative path
fn expand_path(path: &Path) -> PathBuf {
    let raw_path = PathBuf::from(path);
    let home = dirs::home_dir().unwrap().as_os_str().to_owned();
    let expanded_path: PathBuf = raw_path
        .components()
        .map(|c| {
            if c.as_os_str() == "~" {
                Component::Normal(&home)
            } else {
                c
            }
        })
        .collect();

    expanded_path
}

fn create_dir_if_not_exists(path: &Path) -> Result<()> {
    match std::fs::create_dir_all(path) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => {
            bail!(
                "Failed to create the destination directory: directory={} err={}",
                path.to_string_lossy(),
                err
            );
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let opts = Opts::parse();
    let start = std::time::Instant::now();

    let (tx_projects, rx_projects) = tokio::sync::mpsc::channel::<Project>(500);
    let (tx_projects_actions, mut rx_projects_actions) =
        tokio::sync::mpsc::channel::<ProjectAction>(500);

    let tx_projects_actions_1 = tx_projects_actions.clone();
    let expanded_path = expand_path(&opts.directory);
    create_dir_if_not_exists(&expanded_path)
        .with_context(|| "Failed to create directory given on the CLI")?;

    let clone_method = opts.clone_method;
    tokio::spawn(async move {
        if let Err(err) = clone_projects(
            rx_projects,
            tx_projects_actions_1,
            &expanded_path,
            clone_method,
        )
        .await
        {
            log::error!("Failed to clone projects: err={}", err);
        }
    });

    let client = make_http_client(&opts.api_token)?;
    tokio::spawn(async move {
        if let Err(err) = fetch_projects(client, tx_projects, tx_projects_actions, &opts.url).await
        {
            log::error!("Failed to fetch projects: {}", err);
        }
    });

    let mut todo_count: Option<usize> = None;
    let mut total_count = 0usize;
    let mut cloned_count = 0usize;
    let mut total_bytes = 0usize;

    loop {
        let message = rx_projects_actions.recv().await;
        log::debug!("todo_count={:?} message={:?}", todo_count, message);

        match message {
            None => {
                return Ok(());
            }
            Some(ProjectAction::ToClone) => {
                todo_count = todo_count.map(|n| n + 1).or(Some(1));
                total_count += 1;
            }
            Some(ProjectAction::Failed { project_path, err }) => {
                todo_count = todo_count.map(|n| n - 1);
                println!("{} {} ({})", style("❌").red(), project_path, err,);
            }
            Some(ProjectAction::Cloned {
                project_path,
                received_bytes,
                received_objects,
            }) => {
                cloned_count += 1;
                total_bytes += received_bytes;
                todo_count = todo_count.map(|n| n - 1);
                println!(
                    "{} {} ({}, {} objects)",
                    style("✓").green(),
                    project_path,
                    ByteSize(received_bytes as u64),
                    received_objects
                );
            }
        };
        if todo_count.unwrap_or(0) == 0 {
            log::debug!("Done");
            println!(
                "Successfully cloned: {}/{} ({})\nDuration: {:?}",
                cloned_count,
                total_count,
                ByteSize(total_bytes as u64),
                start.elapsed(),
            );
            return Ok(());
        }
    }
}
