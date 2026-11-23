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
$fish -c 'set -l dir (mktemp -d); set -l name kts_$fish_pid; ln -s /usr/bin/sleep $dir/$name; $dir/$name 10 & set -l p $last_pid; set -l q (builtin kill -p $name); test "$q" = "$p"; echo name_pid=$status; builtin kill -TERM $p; command rm -rf $dir'
# CHECK: name_pid=0

# A process name is a signal target.
$fish -c 'set -l dir (mktemp -d); set -l name kts_$fish_pid; ln -s /usr/bin/sleep $dir/$name; $dir/$name 10 & set -l p $last_pid; builtin kill -TERM $name; echo name_signal=$status; builtin kill -KILL $p 2>/dev/null; command rm -rf $dir'
# CHECK: name_signal=0

# The verbose form prints the resolved target on standard output.
$fish -c 'set -l dir (mktemp -d); set -l name kts_$fish_pid; ln -s /usr/bin/sleep $dir/$name; $dir/$name 10 & set -l p $last_pid; builtin kill --verbose -0 $name | string match -q "sending signal 0 to pid *"; echo verbose=$status; builtin kill -TERM $p; command rm -rf $dir'
# CHECK: verbose=0

# A process without a userspace handler is skipped by -r.
$fish -c 'set -l dir (mktemp -d); set -l name kts_$fish_pid; ln -s /usr/bin/sleep $dir/$name; $dir/$name 10 & set -l p $last_pid; builtin kill -r -TERM $name; echo handler=$status; builtin kill -KILL $p; command rm -rf $dir'
# CHECK: handler=1

# -d decodes signal-related /proc status fields.
$fish -c 'builtin kill -d $fish_pid | string match -q "*Caught:*"'
echo decode=$status
# CHECK: decode=0

# A queued signal sends a sigqueue value.
$fish -c 'sleep 10 & set -l p $last_pid; builtin kill -q 7 -TERM $p; echo queue=$status'
# CHECK: queue=0

# The timeout sends its first signal and stops waiting when the process exits.
$fish -c 'sleep 10 & set -l p $last_pid; builtin kill --timeout 100 KILL -TERM $p; echo timeout=$status'
# CHECK: timeout=0

# Queue and pid mode are mutually exclusive.
$fish -c 'builtin kill -p -q 7 1 2>/dev/null; echo conflict=$status'
# CHECK: conflict=1

# A pidfd inode names the same process without a PID race.
#REQUIRES: command -v python3
$fish -c 'sleep 10 & set -l p $last_pid; set -l inode (python3 -c "import os,sys; print(os.fstat(os.pidfd_open(int(sys.argv[1]))).st_ino)" $p); builtin kill -TERM "$p:$inode"; echo pidfdino=$status'
# CHECK: pidfdino=0
