# gitlab-clone-all

Clone locally all git projects from Gitlab. This is handy for example to search locally with `ripgrep` very quickly, or hack on projects.

Every project is cloned concurrently for maximum performance and the memory usage remains under 70MiB even with thousands of big projects.

## Usage

*The api token is optional. Without it, only publicly accessible repositories can be cloned. Go to (https://[your gitlab url]/-/profile/personal_access_tokens) to create one with `read_api` and `read_repository` access*

Usage:

```
Clone all git repositories from gitlab

USAGE:
    gitlab-clone-all [OPTIONS]

OPTIONS:
    -a, --api-token <API_TOKEN>          [default: ]
    -c, --clone-method <CLONE_METHOD>    [default: https] [possible values: https, ssh]
    -d, --directory <DIRECTORY>          Root directory where to clone all the projects [default: .]
    -h, --help                           Print help information
    -u, --url <URL>                      [default: gitlab.com]
```

Build:

```sh
# Requires libgit2 & openssl e.g. `brew install openssl libgit2`
$ cargo build --release
```

Simple usage (the exact output will be different for you):

```sh
$ ./target/release/gitlab-clone-all --directory=/tmp
...
✓ youlysses/pmm-theme (2.5 KB, 6 objects)
✓ dkrikun/someproj (531 B, 5 objects)
✓ naggie/averclock (37.8 KB, 268 objects)
✓ rocksoniko/easy (305 B, 3 objects)
✓ diverops/hello-again (678 B, 6 objects)
✓ leberwurscht/teardownwalls (268.7 KB, 932 objects)
✓ hcs/hcs_utils (250.2 KB, 858 objects)
✓ alessioalex/pushover (428.8 KB, 738 objects)
✓ thanhtam1612/xdpm2010 (1.1 KB, 12 objects)
✓ brad_richards/math-stuff (671.5 KB, 1415 objects)
...

Successfully cloned: 299/300 (1.7 GB)
Duration: 270.068645291s
```

Complex usage:

```sh
$ ./target/release/gitlab-clone-all --api-token <API_TOKEN> --clone-method=ssh --directory=/tmp/ --url=custom.gitlab.com
```

## Development

```sh
# Adapt for your platform
$ brew install rust openssl libgit2

# Optional verbose logs
$ export RUST_LOG=debug

$ cargo run -- --api-token="$GITLAB_API_TOKEN" --directory=/tmp/$(date +%s) --clone-method=ssh
```

## Docker

*Due to limitations of git/Gitlab/the current capabilities of the project, running the app in Docker won't work if authentication is required without extra work (that's because you need to copy your ssh private key into the container).*

```sh
$ docker build -t gitlab-clone-all:latest .
$ docker run -it gitlab-clone-all:latest gitlab-clone-all --directory=/tmp/
```

## Roadmap

- [ ] Retrying
- [ ] `--me` option to only clone my repositories
- [ ] Stop if no project could be fetched from the Gitlab API at all
- [ ] Max network rate CLI option
