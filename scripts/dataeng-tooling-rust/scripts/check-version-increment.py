#!/usr/bin/env python
#
# Check that a version number has correctly increased.
# Called like:
#
#    ./scripts/check-version-increment.py OLD NEW
#
# This script will exit successfully if the new version number is bigger
# than the old one.
#
# This one's in python rather than shell because it makes it much easier
# to do looping numerical comparisons, and the GitHub Actions runner image
# has python pre-installed.

import sys


def parse_version(version):
    """Return the individual numeric parts of a version number."""
    # Recall that the version format is "M.NN-X.Y".
    parts = tuple(int(v) for vs in version.split("-") for v in vs.split("."))
    if len(parts) != 4:
        raise ValueError(f"Invalid version number: {version}")
    return parts


def check_version_increment(old_version, new_version):
    """Check that new_version is strictly greater than old_version."""
    # Recall that the version format is "M.NN-X.Y".
    old_parts = parse_version(old_version)
    new_parts = parse_version(new_version)
    for (old, new) in zip(old_parts, new_parts):
        if new > old:
            return True
        if new < old:
            raise ValueError(f"Version {new_version} is less than {old_version}")
    raise ValueError(f"The version number has not changed: {old_version}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise RuntimeError("Usage: ./scripts/check-version-increment.py OLD NEW")
    check_version_increment(sys.argv[1], sys.argv[2])
