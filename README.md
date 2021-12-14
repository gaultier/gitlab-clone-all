# gitlab_clone_all

Clone locally all git projects from Gitlab. 

## Usage

*The api token is optional. Without it, only publicly accessible repositories can be cloned.*

```sh
# Requires libgit2 & openssl e.g. `brew install openssl libgit2`
$ cargo build --release

$ ./target/release/gitlab-clone-all --help
$ ./target/release/gitlab-clone-all --api-token <API_TOKEN> --clone-method=https --directory=/tmp/ --url=custom.gitlab.com
```


## Development

```sh
# Optional
$ export RUST_LOG=debug

$ cargo r -- --api-token="$GITLAB_API_TOKEN" --directory=/tmp/$(date +%s) --clone-method=ssh
```


## Roadmap


- [ ] Retrying
