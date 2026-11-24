# RUN: fish=%fish %fish %s

# A job that fails during job population never executes, so the profile record omits
# it; a job that fails while launching its redirection does execute, so its record stays.

set -l profile (mktemp)
$fish --no-config --profile $profile -c '
    set -g fish_failglob 1
    echo *.no_match_at_all_xyzzy
    echo hi > /no_such_dir_xyzzy/out
' 2>/dev/null

grep -c no_match_at_all_xyzzy $profile
# CHECK: 0

grep -c no_such_dir_xyzzy $profile
# CHECK: 1

command rm $profile
