#RUN: fish=%fish %fish %s

# Tests for the kill builtin's signal namespace and target statuses.

# Signal names, numbers and the 128+signal convention.
$fish -c 'builtin kill -l 15'
# CHECK: TERM
$fish -c 'builtin kill -l 143'
# CHECK: TERM
$fish -c 'builtin kill -l 0'
# CHECK: 0
$fish -c 'builtin kill -l 34'
# CHECK: RT0
$fish -c 'builtin kill -l 0x4000'
# CHECK: TERM
$fish -c 'builtin kill -l HUP'
# CHECK: HUP
$fish -c 'builtin kill -L | string match -q " 1 HUP*"'
echo table=$status
# CHECK: table=0

# A null signal checks a live child; a terminating signal then ends it.
$fish -c 'sleep 10 & set -l p $last_pid; builtin kill -0 $p; echo null=$status; builtin kill -TERM $p; echo term=$status'
# CHECK: null=0
# CHECK: term=0

# One good target and one missing target are a partial success.
$fish -c 'sleep 10 & set -l p $last_pid; builtin kill -0 $p 2147483647 2>/dev/null; echo partial=$status; builtin kill -TERM $p'
# CHECK: partial=64

# All failed targets and invalid signal names fail.
$fish -c 'builtin kill -0 2147483647 2>/dev/null; echo missing=$status'
# CHECK: missing=1
$fish -c 'builtin kill -s BOGUS 1 2>/dev/null; echo bad=$status'
# CHECK: bad=1

# The userland function expands a job specification before invoking the builtin.
$fish -c 'sleep 10 & kill %1; echo job=$status'
# CHECK: job=0
