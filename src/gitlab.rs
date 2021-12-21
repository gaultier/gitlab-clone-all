use crate::project::ProjectAction;
use anyhow::{Context, Result};
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: u64,
    pub ssh_url_to_repo: String,
    pub http_url_to_repo: String,
    pub path_with_namespace: String,
}

async fn fetch_projects_paginated(
    client: reqwest::Client,
    project_id_after: Option<u64>,
    gitlab_url: &str,
) -> Result<Vec<Project>> {
    let  req = client
        .get(format!("{}/api/v4/projects?statistics=false&top_level=&with_custom_attributes=false&all_available=true&top_level&order_by=id&sort=asc&pagination=keyset&per_page=100&id_after={}", gitlab_url, project_id_after.unwrap_or(0)));

    let json = req.send().await?.text().await?;

    let projects: Vec<Project> = serde_json::from_str(&json)
        .with_context(|| format!("Failed to parse projects from JSON: json={}", json))?;

    log::debug!("Fetched projects: count={}", projects.len(),);
    Ok(projects)
}

pub fn make_http_client(api_token: &str) -> Result<Client> {
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

pub async fn fetch_projects(
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

#[cfg(test)]
mod tests {
    use super::*;
    use warp::Filter;

    #[tokio::test]
    async fn foo() {
        let hello = warp::path!("api" / "v4" / "projects").map(|| {
            r#"
                [
                 {
                    "id": 3,
                    "ssh_url_to_repo": "ssh://A",
                    "http_url_to_repo": "http://B",
                    "path_with_namespace": "C/D"
                 }   
                ]
                "#
        });

        tokio::spawn(async move {
            warp::serve(hello).run(([127, 0, 0, 1], 8123)).await;
        });

        let client = reqwest::Client::new();
        let projects = fetch_projects_paginated(client, None, "http://localhost:8123")
            .await
            .unwrap();

        assert_eq!(projects.len(), 1);
        assert_eq!(
            projects[0],
            Project {
                id: 3,
                ssh_url_to_repo: String::from("ssh://A"),
                http_url_to_repo: String::from("http://B"),
                path_with_namespace: String::from("C/D"),
            }
        );
    }
}