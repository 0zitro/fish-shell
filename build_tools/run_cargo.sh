#!/bin/sh
# Runs cargo for the build tree, supplying the build version cargo cannot
# derive itself.
#
# Cargo tracks the inputs a build script declares.  The version comes from the
# checkout's `git describe`, which moves with the refs and with the worktree, so
# no declared path witnesses it.  Computing it here, once per cargo invocation,
# makes it an environment value cargo compares among the build script's inputs.
#
# Usage: run_cargo.sh <cargo> <arguments...>

set -eu

cargo=$1
shift

FISH_BUILD_VERSION=$("$(dirname "$0")/git_version_gen.sh")
export FISH_BUILD_VERSION

exec "$cargo" "$@"
