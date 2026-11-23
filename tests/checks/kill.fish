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

# A process name resolves to the matching process identifier without signalling.
$fish -c 'set -l dir (mktemp -d); ln -s /usr/bin/sleep $dir/kill_test_slp; $dir/kill_test_slp 10 & set -l p $last_pid; set -l q (builtin kill -p kill_test_slp); test "$q" = "$p"; echo name_pid=$status; builtin kill -TERM $p; command rm -rf $dir'
# CHECK: name_pid=0

# A process name is a signal target.
$fish -c 'set -l dir (mktemp -d); ln -s /usr/bin/sleep $dir/kill_test_slp; $dir/kill_test_slp 10 & set -l p $last_pid; builtin kill -TERM kill_test_slp; echo name_signal=$status; builtin kill -KILL $p 2>/dev/null; command rm -rf $dir'
# CHECK: name_signal=0

# The verbose form prints the resolved target on standard output.
$fish -c 'set -l dir (mktemp -d); ln -s /usr/bin/sleep $dir/kill_test_slp; $dir/kill_test_slp 10 & set -l p $last_pid; builtin kill --verbose -0 kill_test_slp | string match -q "sending signal 0 to pid *"; echo verbose=$status; builtin kill -TERM $p; command rm -rf $dir'
# CHECK: verbose=0

# A process without a userspace handler is skipped by -r.
$fish -c 'set -l dir (mktemp -d); ln -s /usr/bin/sleep $dir/kill_test_slp; $dir/kill_test_slp 10 & set -l p $last_pid; builtin kill -r -TERM kill_test_slp; echo handler=$status; builtin kill -KILL $p; command rm -rf $dir'
# CHECK: handler=1

# -d decodes signal-related /proc status fields.
$fish -c 'builtin kill -d $fish_pid | string match -q "*Caught:*"'
echo decode=$status
# CHECK: decode=0
