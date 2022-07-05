#!/usr/bin/env bash
#
# Check that a PR is properly incrementing the version number.
#
# We use this as a check in GitHub Actions to ensure that every
# commit landing on `main` has a corresponding changelog entry
# and version number, allowing us to automatically build and publish
# and image from it as soon as it lands.
#

set -e

THIS_VERSION=`./scripts/version-number.sh`
MAIN_VERSION=`./scripts/version-number.sh ${GITHUB_BASE_SHA-main}`

if [ -z "${THIS_VERSION}" ]; then
    echo "ERROR: no version number for the current checkout" 1>&2
    exit 1
fi

if [ -z "${MAIN_VERSION}" ]; then
    echo "ERROR: no version number for the main branch" 1>&2
    exit 1
fi

if ./scripts/check-version-increment.py "${MAIN_VERSION}" "${THIS_VERSION}"; then true; else
    echo ""
    echo "Please update CHANGELOG.md with an appropriate new version number"
    echo "and a description of what has changed, so that we can automatically"
    echo "publish this change once it is merged."
    echo ""
    exit 1
fi

THIS_RUST_VERSION=`echo ${THIS_VERSION} | cut -s -d '-' -f 1`
if grep "^FROM rust:${THIS_RUST_VERSION}" Dockerfile > /dev/null; then true; else
    echo ""
    echo "The first component of the version number in CHANGELOG.md does not match"
    echo "the version of Rust used to build the Docker image. Please update the"
    echo "version numbers to match before merging."
    echo ""
    exit 1
fi

echo "Looks good!"
