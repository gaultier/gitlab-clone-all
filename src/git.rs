use crate::project::*;
use anyhow::Context;
use anyhow::Result;
use git2::build::CheckoutBuilder;
use git2::{Cred, ErrorCode, RemoteCallbacks};
use std::cell::RefCell;
use std::path::Path;
use std::str::FromStr;
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum CloneMethod {
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

pub async fn clone_projects(
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
