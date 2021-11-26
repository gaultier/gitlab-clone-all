use anyhow::{Context, Result};
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Deserialize)]
struct Group {
    id: u64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Project {
    id: u64,
    ssh_url_to_repo: String,
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

#[tokio::main]
async fn main() -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Project>(500);
    tokio::spawn(async move {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let root_dir_path = PathBuf::new().join("/tmp").join(now.to_string());
        std::fs::create_dir(&root_dir_path).unwrap();

        while let Some(project) = rx.recv().await {
            let path = root_dir_path.join(&project.path_with_namespace);
            println!("Received project {:?} fs_path={:?}", &project, &path);
        }
    });

    let token = std::env::var("GITLAB_TOKEN").unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("PRIVATE-TOKEN", HeaderValue::from_str(&token).unwrap());

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .default_headers(headers)
        .build()
        .unwrap();

    let groups = fetch_groups(&client).await?;
    println!("Groups: {:?}", groups);

    let join_handles = groups
        .into_iter()
        .map(|group| {
            let client = client.clone();
            let tx = tx.clone();

            tokio::spawn(async move {
                let mut project_id_after = None;
                loop {
                    let res =
                        fetch_group_projects(client.clone(), group.id, project_id_after).await;
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
