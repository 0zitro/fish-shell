#!/bin/sh
# Originally from the git sources (GIT-VERSION-GEN)

set -e

# Find the fish directory as two levels up from script directory.
FISH_BASE_DIR="$( cd "$( dirname "$( dirname "$0" )" )" && pwd )"

version=$(
    awk <"$FISH_BASE_DIR/Cargo.toml" -F'"' '$1 == "version = " { print $2 }'
)

# The version names the commit.  `git describe` spells the tag it picked, and a
# tag outside the release scheme would put its name into the version, so only
# tags that spell a release version are considered and the string is built from
# the commit count and the abbreviated commit.
if describe=$(
    GIT_CEILING_DIRECTORIES=$FISH_BASE_DIR/.. \
        git -C "$FISH_BASE_DIR" describe --always --long --dirty \
            --match '[0-9]*.[0-9]*.[0-9]*' 2>/dev/null); then
    dirty=
    case "$describe" in
        *-dirty)
            dirty=-dirty
            describe=${describe%-dirty}
            ;;
    esac

    hash=${describe##*-g}
    if [ "$hash" = "$describe" ]; then
        # No tag matched, so describe printed the abbreviated commit.
        version=$version-g$hash$dirty
    else
        rest=${describe%-g"$hash"}
        count=${rest##*-}
        tag=${rest%-"$count"}
        if [ "$tag" = "$version" ] && [ "$count" = 0 ] && [ -z "$dirty" ]; then
            # The commit is the release tag itself.
            :
        else
            version=$version-$count-g$hash$dirty
        fi
    fi
fi

echo "$version"
