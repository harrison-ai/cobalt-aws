#!/usr/bin/env bash
#
# Publish the current git HEAD to dockerhub, if it's the main branch.
#

set -e

IMAGE="harrisonai/rust"

# We only ever publish from a clean checkout of `main`.
if git diff --exit-code > /dev/null; then true; else 
    echo "Refusing to publish from a working dir with unstaged changes"
    exit 1
fi
if git diff --cached --exit-code > /dev/null; then true; else
    echo "Refusing to publish from a working dir with uncommited changes"
    exit 1
fi
if [ `git branch --show-current` != "main" ]; then
    echo "Refusing to publish from a branch other than 'main'"
    exit 1
fi

VERSION=`./scripts/version-number.sh`
RUST_VERSION=`echo ${VERSION} | cut -s -d '-' -f 1`
OUR_MAJOR_VERSION="$RUST_VERSION-`echo ${VERSION} | cut -s -d '-' -f 2 | cut -s -d '.' -f 1`"
echo "Building image with tags: '${VERSION}', '${OUR_MAJOR_VERSION}', '${RUST_VERSION}', 'latest'"

docker build -t "${IMAGE}:latest" \
    -t "${IMAGE}:${VERSION}" \
    -t "${IMAGE}:${OUR_MAJOR_VERSION}" \
    -t "${IMAGE}:${RUST_VERSION}" \
    .

docker push "${IMAGE}:latest"
docker push "${IMAGE}:${VERSION}"
docker push "${IMAGE}:${OUR_MAJOR_VERSION}"
docker push "${IMAGE}:${RUST_VERSION}"
