#!/usr/bin/env bash
#
# Get the latest version number in the changelog file in git.
#
# This is a little trick to help ensure we maintain a changelog and think
# breaking changes up front, and also to help us easily publish new images
# built from commits as soon as they merge to master.
#

set -e

# By default this script will read the latest version number from the changelog
# in the git `HEAD` ref, but you can pass a specific git ref as first argument.
GITREF="${1-HEAD}"

# In words:
#  * read the changelog at the specified ref
#  * find all markdown subheadings that contain just a version number
#  * take the first one
#  * delete all the non-version-number characters
VERSION=`git show $GITREF:CHANGELOG.md | grep "^## [0-9]\+\.[0-9]\+-[0-9]\+\.[0-9]\+$" | head -n 1 | tr -d "# "`

if [ -z $VERSION ]; then
    echo "Error: Unable to find version number in changelog" 1>&2
    exit 1
fi

echo $VERSION
