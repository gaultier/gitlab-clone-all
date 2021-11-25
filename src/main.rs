use anyhow::{Context, Result};
use serde::Deserialize;
use std::env::VarError;

#[derive(Debug, Deserialize)]
struct Group {
    id: u64,
    name: String,
}

async fn fetch_groups(
    client: reqwest::Client,
    token: Result<String, VarError>,
) -> Result<Vec<Group>> {
    let mut req = client.get("https://gitlab.ppro.com/api/v4/groups?all_available&top_level");
    if let Ok(token) = token {
        req = req.header("PRIVATE-TOKEN", token);
    }

    let json = req.send().await?.text().await?;

    let groups: Vec<Group> = serde_json::from_str(&json).context("failed to parse to JSON")?;

    Ok(groups)
}

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("GITLAB_TOKEN");
    let client = reqwest::Client::new();

    let groups = fetch_groups(client, token).await?;
    println!("Groups: {:#?}", groups);

    Ok(())
}
