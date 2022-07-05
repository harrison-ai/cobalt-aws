# Rust build and development tooling for harrison.ai

This repository builds the [`harrisonai/rust`](https://hub.docker.com/r/harrisonai/rust)
docker image, a convenient collection of Rust build and development tooling
for working with Rust-based projects in the harrison.ai Data Engineering team.

The docker image provides:

* Rust, rustup, cargo and friends, like you'd get from a standard Rust dev image.
* Pre-configured components and settings to cross-compile for `aarch64` targets,
  for deployment to AWS
* Pre-configured components and settings to cross-compile static binaries for x86\_64-unknown-linux targets,
  with zlib and openssl statically compiled.
* Useful project-lifecycle-management tools such as `cargo deny` for auditing
  dependency licenses, `cargo about` for generating a license summary, and
  `cargo release` for cutting releases.
* A custom `cargo hai-all-checks` command to easily run our standard suite of
  quality checks from a CI environment.

## Using the image

A typical workflow would be to mount your working directory in the image
and then run various `cargo` build commands, like this:

```sh
docker run
    # Automatically delete the container when it's finished running
    --rm
    # Mount your current working directory into the container
    --volume `pwd`:/app
    # Run commands in your current working directory
    --workdir /app
    # Use the published rust tooling image
    harrisonai/rust
    # To run various cargo build commands
    cargo build
```

In repositories using the [three-musketeers pattern](https://3musketeers.io/)
you can conveniently automate the environment setup in your `docker-compose.yml`
like this:

```
services:
  cargo:
    image: harrisonai/rust:{version-tag}
    entrypoint: cargo
    volumes:
      - '~/.cargo/registry:/usr/local/cargo/registry'
      - '.:/app'
    working_dir: '/app'
```

You can then invoke `cargo` via `docker-compose` like this:

```
docker-compose run --rm cargo build
```

We plan to publish a cookiecutter Rust project using this pattern, once we've
stabilised some of the details.

## Tags and Versioning

In an attempt to minimise possible confusion for users, the docker image version
reflects the underlying Rust version down to semver-minor level and appends a
separate version number for the customizations that we layer on top. The general
form is `harrisonai/rust:M.NN-X.Y` where:

* `M.NN` is the semver-minor version of Rust included in the image.
* `X.Y` is a major.minor version number for changes in this repo.

So for example, `harrisonai/rust:1.56-1.3` would include the Rust toolchain at
some version in the `1.56` series, and be the third release of our customizations
on top of that series.

Every commit merged to `main` in this repo creates a new version that is automatically
built and pushed to dockerhub, with version number updated according to the following
rules:

* If the change were purely additive (e.g. installing some additional tools, or a new
  point release of Rust) then it would move the version from `1.56-1.3` to `1.56-1.4`.
* If the change had the potential for breakage (such as replacing `cargo-deny` with a
  different tool) then it would move the version from `1.56-1.3` to `1.56-2.0`.
* If the change updated to a new major or minor version of Rust, then it would move
  the version from `1.56-1.3` to `1.57-1.0`.

We also provide floating semver tags if you don't want to pin to a specific release:

* `harrisonai/rust:M.NN-X` tracks the latest additive updates but should never pull
  in any breaking changes.
* `harrisonai/rust:M.NN` tracks all updates other than Rust version changes, and might
  potentially receive breaking changes in the surrounding tooling.

But please note that assessment of possible breaking changes is based purely on a
best-effort human-in-the-loop basis.

The scripting to manage all this is under [`./scripts`](./scripts) and is orchestrated
by GitHub Actions.
