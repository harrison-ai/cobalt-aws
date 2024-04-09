FROM ghcr.io/harrison-ai/rust:1.77-1.0

# cargo deny version 0.14.3 in the rust base layer fails on cargo deny check with:
# failed to fetch advisory database https://github.com/RustSec/advisory-db: An IO error occurred when talking to the server: error sending request for url (https://github.com/RustSec/advisory-db/info/refs?service=git-upload-pack): error trying to connect: invalid URL, scheme is not http
#
#Re Installing 0.14.3 or 0.14.2 results in this error on cargo deny check:
#internal error: entered unreachable code: unable to find dependency valuable for tracing-core 0.1.32 (registry+https://github.com/rust-lang/crates.io-index) [
#    NodeDep {
#        name: "once_cell",
#        pkg: PackageId {
#            repr: "once_cell 1.19.0 (registry+https://github.com/rust-lang/crates.io-index)",
#        },
#        dep_kinds: [
#            DepKindInfo {
#                kind: Normal,
#                cfg: None,
#            },
#        ],
#    },
#]
#note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
#
#Once this version has been shown to work the version of cargo-deny can be
#downgraded in the harrison-ai/rust:1.75 image and this dockerfile removed.
RUN cargo install -f --version 0.14.1 cargo-deny --no-default-features
