# gitlab-clone-all

Clone all projects from Gitlab locally.

## Usage

*The api token is optional. Without it, only publicly accesible repositories can be cloned.*

```sh
$ cargo build --release

$ ./target/release/gitlab-clone-all --help
$ ./target/release/gitlab-clone-all --api-token <API_TOKEN> --clone-method=https --directory=/tmp/
```
