use anyhow::{Context, Result};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};

#[derive(Debug, Deserialize)]
struct Group {
    id: u64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Project {
    id: u64,
    ssh_url_to_repo: String,
}

async fn fetch_groups(client: &reqwest::Client, token: &str) -> Result<Vec<Group>> {
    let mut req = client
        .get("https://gitlab.ppro.com/api/v4/groups?statistics=false&top_level=&with_custom_attributes=false&all_available=true&top_level&order_by=id&sort=asc&pagination=keyset&per_page=100"); // TODO: pagination
    req = req.header("PRIVATE-TOKEN", token);

    let json = req.send().await?.text().await?;

    let groups: Vec<Group> = serde_json::from_str(&json).context("failed to parse to JSON")?;

    Ok(groups)
}

async fn fetch_group_projects(
    client: reqwest::Client,
    token: &str,
    group_id: u64,
    project_id_after: Option<u64>,
) -> Result<Vec<Project>> {
    let mut req = client
        .get(format!("https://gitlab.ppro.com/api/v4/groups/{}/projects?statistics=false&top_level=&with_custom_attributes=false&all_available=true&top_level&order_by=id&sort=asc&pagination=keyset&per_page=100&id_after={}", group_id, project_id_after.unwrap_or(0)));
    req = req.header("PRIVATE-TOKEN", token);

    let json = req.send().await?.text().await?;

    let projects: Vec<Project> = serde_json::from_str(&json).context("failed to parse to JSON")?;

    Ok(projects)
}

#[tokio::main]
async fn main() -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Project>(500);
    tokio::spawn(async move {
        while let Some(project) = rx.recv().await {
            println!("Received project {:?}", project);
            // TODO
        }
    });

    let token = Arc::new(std::env::var("GITLAB_TOKEN").unwrap());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let groups = fetch_groups(&client, &token).await?;
    println!("Groups: {:?}", groups);

    let join_handles = groups
        .into_iter()
        .map(|group| {
            let client = client.clone();
            let token = token.clone();
            let tx = tx.clone();

            tokio::spawn(async move {
                let mut project_id_after = None;
                loop {
                    let res =
                        fetch_group_projects(client.clone(), &token, group.id, project_id_after)
                            .await;
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

                            if new_project_id_after == project_id_after
                                || new_project_id_after.is_none()
                            {
                                break;
                            }
                            project_id_after = new_project_id_after;
                        }
                    }
                }
            })
        })
        .collect::<Vec<_>>();
    futures::future::join_all(join_handles).await;
    Ok(())
}
