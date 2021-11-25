use anyhow::{Context, Result};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct Group {
    id: u64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Project {
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
) -> Result<Vec<Project>> {
    let mut req = client
        .get(format!("https://gitlab.ppro.com/api/v4/groups/{}/projects?statistics=false&top_level=&with_custom_attributes=false&all_available=true&top_level&order_by=id&sort=asc&pagination=keyset&per_page=100", group_id)); // TODO: pagination
    req = req.header("PRIVATE-TOKEN", token);

    let json = req.send().await?.text().await?;

    let projects: Vec<Project> = serde_json::from_str(&json).context("failed to parse to JSON")?;

    Ok(projects)
}

#[tokio::main]
async fn main() -> Result<()> {
    let token = Arc::new(std::env::var("GITLAB_TOKEN").unwrap());
    let client = reqwest::Client::new();

    let groups = fetch_groups(&client, &token).await?;
    println!("Groups: {:#?}", groups);

    for group in groups {
        let c = client.clone();
        let t = token.clone();
        tokio::spawn(async move {
            let _ = fetch_group_projects(c, &t, group.id)
                .await
                .map_err(|err| {
                    eprintln!("Err: group_id={} err={}", group.id, err);
                })
                .map(|projects| {
                    for project in projects {
                        println!("group_id={} project={}", group.id, project.ssh_url_to_repo);
                    }
                });
        });
    }
    let notify = tokio::sync::Notify::new();
    notify.notified().await;
    Ok(())
}
