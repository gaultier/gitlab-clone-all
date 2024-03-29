FROM rust:buster as builder
RUN apt update && apt install libgit2-27 -y
WORKDIR /gitlab_clone_all
COPY . .
RUN cargo install --path .

FROM debian:buster
RUN apt update -y && apt install libssl1.1 ca-certificates -y
COPY --from=builder /usr/local/cargo/bin/gitlab-clone-all /usr/local/bin/
CMD ["gitlab-clone-all"]
