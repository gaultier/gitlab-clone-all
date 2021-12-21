use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Project {
    pub id: u64,
    pub ssh_url_to_repo: String,
    pub http_url_to_repo: String,
    pub path_with_namespace: String,
}

#[derive(Debug)]
pub enum ProjectAction {
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
