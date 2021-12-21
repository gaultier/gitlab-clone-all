use anyhow::{bail, Context, Result};
use bytesize::ByteSize;
use clap::Parser;
use console::style;
use gitlab_clone_all::git::*;
use gitlab_clone_all::gitlab::*;
use gitlab_clone_all::project::*;
use std::{path::Component, path::Path, path::PathBuf};

/// Clone all git repositories from gitlab
#[derive(Parser)]
struct Opts {
    /// Root directory where to clone all the projects
    #[clap(short, long, default_value = ".")]
    directory: PathBuf,
    #[clap(short, long, default_value = "")]
    api_token: String,
    #[clap(short, long, default_value = "https://gitlab.com")]
    url: String,
    #[clap(short, long, possible_values = &["https","ssh"], default_value="https")]
    clone_method: CloneMethod,
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
    let opts = {
        let mut opts = Opts::parse();
        if !opts.url.starts_with("http") {
            opts.url = format!("https://{}", opts.url);
        }
        opts
    };

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
            log::error!("Failed to fetch projects: {:?}", err);
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
