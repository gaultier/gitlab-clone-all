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
