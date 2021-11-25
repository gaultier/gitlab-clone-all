use git2::{Cred, Error, RemoteCallbacks};
use std::env;
use std::path::Path;

fn main() {
    let repository_url = std::env::args().nth(1).unwrap();
    // Prepare callbacks.
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_url, username_from_url, _allowed_types| {
        Cred::ssh_key(
            username_from_url.unwrap(),
            None,
            std::path::Path::new(&format!("{}/.ssh/id_rsa_gitlab", env::var("HOME").unwrap())),
            None,
        )
    });

    // Prepare fetch options.
    let mut fo = git2::FetchOptions::new();
    fo.remote_callbacks(callbacks);

    // Prepare builder.
    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fo);

    // Clone the project.
    let repo = builder
        .clone(&repository_url, Path::new("/tmp/foobar"))
        .unwrap();

    println!("Repo head: {:?}", repo.head().unwrap().name())
}
