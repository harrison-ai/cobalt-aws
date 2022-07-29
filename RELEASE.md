## Release Process

To make a release of `cobalt-aws`, follow these steps:

1. Prepare a release PR:
   -  Update the version in `Cargo.toml` to the target version
   -  Update `CHANGELOG.md` by changing the "Unreleased" heading to the target version and creating a blank Unreleased section
   -  Update the `licenses/licenses.html` file by running `make licenses`.
   -  Review the PR history since the previous release and ensure the changelog contents are up to date and correct.
3. Push PR and merge into `main`
   - Trigger publish from Github actions (TODO), or
   - Pull from `main` and publish from local
   - `cargo login <token>`
   - `cargo publish --dry-run`
   - `cargo package --list`
   - `cargo publish`
5. Tag a release on github from https://github.com/harrison-ai/cobalt-aws/releases/new
   - the tag should be in the form v1.2.3
   - The release title should include the version and the human readable date, e.g. "1.2.3 (January 1, 2022)"
   - The release notes should generally be a cut/paste from the Changelog.

Reference: https://doc.rust-lang.org/cargo/reference/publishing.html

## Note

These instructions are a work in progress, we will be refining and automating them as we go.
