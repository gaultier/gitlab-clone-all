use crate::project::{Project, ProjectAction};
use anyhow::{Context, Result};
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use reqwest::Client;
use std::time::Duration;
use tokio::sync::mpsc::Sender;

async fn fetch_projects_paginated(
    client: reqwest::Client,
    project_id_after: Option<u64>,
    gitlab_url: &str,
) -> Result<Vec<Project>> {
    let  req = client
        .get(format!("{}/api/v4/projects?statistics=false&top_level=&with_custom_attributes=false&all_available=true&top_level&order_by=id&sort=asc&pagination=keyset&per_page=100&id_after={}", gitlab_url, project_id_after.unwrap_or(0)));
    log::debug!("project_id_after={:?}", project_id_after);

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
        if new_project_id_after == project_id_after || new_project_id_after.is_none() {
            break;
        }
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

        project_id_after = new_project_id_after;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use warp::Filter;

    #[tokio::test]
    async fn one_page() {
        env_logger::init();

        let res = [Project {
            id: 3,
            ssh_url_to_repo: String::from("ssh://A"),
            http_url_to_repo: String::from("http://B"),
            path_with_namespace: String::from("C/D"),
        }];

        let res1 = res.clone();
        let projects_route =
            warp::path!("api" / "v4" / "projects").map(move || Ok(warp::reply::json(&res1)));

        tokio::spawn(async move {
            warp::serve(projects_route)
                .run(([127, 0, 0, 1], 8123))
                .await;
        });

        let client = reqwest::Client::new();
        let (tx_projects, mut rx_projects) = tokio::sync::mpsc::channel::<Project>(1);
        let (tx_projects_actions, mut rx_projects_actions) =
            tokio::sync::mpsc::channel::<ProjectAction>(1);
        fetch_projects(
            client,
            tx_projects,
            tx_projects_actions,
            "http://localhost:8123",
        )
        .await
        .unwrap();

        let project = rx_projects.recv().await.unwrap();
        assert_eq!(project, res[0]);
        assert_eq!(rx_projects.recv().await, None);

        let action = rx_projects_actions.recv().await.unwrap();
        assert_eq!(action, ProjectAction::ToClone);
        assert_eq!(rx_projects_actions.recv().await, None);
    }

    #[tokio::test]
    async fn two_pages() {
        let first_page = [Project {
            id: 1,
            ssh_url_to_repo: String::from("ssh://A"),
            http_url_to_repo: String::from("http://B"),
            path_with_namespace: String::from("C/D"),
        }];
        let second_page = [Project {
            id: 2,
            ssh_url_to_repo: String::from("ssh://A"),
            http_url_to_repo: String::from("http://B"),
            path_with_namespace: String::from("C/D"),
        }];

        let first_page1 = first_page.clone();
        let second_page1 = second_page.clone();
        let projects_route = warp::get()
            .and(warp::path!("api" / "v4" / "projects"))
            .and(warp::query::<HashMap<String, String>>())
            .map(
                move |p: HashMap<String, String>| match p.get("id_after").map(|s| s.as_str()) {
                    Some("1") | Some("2") => Ok(warp::reply::json(&second_page1)),
                    Some("0") | None => Ok(warp::reply::json(&first_page1)),
                    Some(id) => panic!("Unkown id_after={}", id),
                },
            );

        tokio::spawn(async move {
            warp::serve(projects_route)
                .run(([127, 0, 0, 1], 8124))
                .await;
        });

        let client = reqwest::Client::new();
        let (tx_projects, mut rx_projects) = tokio::sync::mpsc::channel::<Project>(2);
        let (tx_projects_actions, mut rx_projects_actions) =
            tokio::sync::mpsc::channel::<ProjectAction>(2);
        fetch_projects(
            client,
            tx_projects,
            tx_projects_actions,
            "http://localhost:8124",
        )
        .await
        .unwrap();

        {
            // First page
            let project = rx_projects.recv().await.unwrap();
            assert_eq!(project, first_page[0]);

            let action = rx_projects_actions.recv().await.unwrap();
            assert_eq!(action, ProjectAction::ToClone);
        }
        {
            // Second page
            let project = rx_projects.recv().await.unwrap();
            assert_eq!(project, second_page[0]);
            assert_eq!(rx_projects.recv().await, None);

            let action = rx_projects_actions.recv().await.unwrap();
            assert_eq!(action, ProjectAction::ToClone);
            assert_eq!(rx_projects_actions.recv().await, None);
        }
    }
}
