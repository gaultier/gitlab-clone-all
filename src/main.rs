use anyhow::{bail, Context, Result};
use clap::Parser;
use git2::{Cred, RemoteCallbacks};
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use reqwest::Client;
use serde::Deserialize;
use std::str::FromStr;
use std::{path::PathBuf, time::Duration};
use tokio::sync::mpsc::{Receiver, Sender};

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
    name: String,
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

    let groups: Vec<Group> = serde_json::from_str(&json).context("failed to parse to JSON")?;

    Ok(groups)
}

async fn fetch_group_projects(
    client: reqwest::Client,
    group_id: u64,
    project_id_after: Option<u64>,
) -> Result<Vec<Project>> {
    let  req = client
        .get(format!("https://gitlab.ppro.com/api/v4/groups/{}/projects?statistics=false&top_level=&with_custom_attributes=false&all_available=true&top_level&order_by=id&sort=asc&pagination=keyset&per_page=100&id_after={}", group_id, project_id_after.unwrap_or(0)));

    let json = req.send().await?.text().await?;

    let projects: Vec<Project> = serde_json::from_str(&json).context("failed to parse to JSON")?;

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
        .timeout(Duration::from_secs(10))
        .default_headers(headers)
        .build()
        .with_context(|| "Failed to create http client")
}

async fn clone_projects(mut rx: Receiver<Project>, opts: &Opts) -> Result<()> {
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

    while let Some(project) = rx.recv().await {
        let path = opts.directory.join(&project.path_with_namespace);
        println!("Received project {:?} fs_path={:?}", &project, &path);

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

        match builder.clone(&project.http_url_to_repo, &path) {
            Ok(_repo) => {
                println!("Cloned project={:?}", &project);
            }
            Err(e) => eprintln!("Failed to clone: project={:?} err={}", &project, e),
        };
    }
    Ok(())
}

async fn fetch_all_projects_for_group(client: reqwest::Client, tx: Sender<Project>, group: Group) {
    let mut project_id_after = None;
    loop {
        let res = fetch_group_projects(client.clone(), group.id, project_id_after).await;
        match res {
            Err(err) => {
                eprintln!("Err: group_id={} err={}", group.id, err);
                break;
            }
            Ok(projects) => {
                let new_project_id_after = projects.iter().map(|p| p.id).last();
                for project in projects {
                    println!("group_id={} project={:?}", group.id, project);
                    tx.send(project).await.unwrap();
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
    let opts: Opts = Opts::parse();

    let (tx, rx) = tokio::sync::mpsc::channel::<Project>(500);
    let client = make_http_client(&opts.api_token)?;
    let _: Result<()> = tokio::spawn(async move {
        let groups = fetch_groups(&client).await?;
        println!("Groups: {:?}", groups);

        let join_handles = groups
            .into_iter()
            .map(|group| {
                let client = client.clone();
                let tx = tx.clone();

                tokio::spawn(async move {
                    fetch_all_projects_for_group(client, tx, group).await;
                })
            })
            .collect::<Vec<_>>();
        futures::future::join_all(join_handles).await;

        Ok(())
    })
    .await?;

    clone_projects(rx, &opts).await
}
