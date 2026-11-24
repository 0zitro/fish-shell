# RUN: %fish %s

# A background block is a job: jobs lists it, wait with its job spec joins it, and the
# operand-less wait waits for it.
begin
    sleep 0.05
    echo done
end &
set -l listing (jobs)
string match -q '*running*' $listing
echo listed=$status
# CHECK: listed=0
wait %1
echo waited=$status
# CHECK: done
# CHECK: waited=0

# A task job that starts no process leaves the starting task's last_pid unchanged and the
# task's own last_pid empty.
set -l before $last_pid
begin
    echo task_last_pid=(count $last_pid)
end &
test "$last_pid" = "$before"
echo last_pid_unchanged=$status
wait
# CHECK: last_pid_unchanged=0
# CHECK: task_last_pid=0

# Two tasks that block on child processes interleave.
begin
    sleep 0.2
    echo a
end &
begin
    sleep 0.1
    echo b
end &
wait
# CHECK: b
# CHECK: a

# wait -n returns the status of the first job that finishes.
begin
    sleep 0.2
    exit 3
end &
begin
    sleep 0.1
    exit 4
end &
wait -n
echo first=$status
wait
# CHECK: first=4
wait -n
echo no_jobs=$status
# CHECK: no_jobs=0

# A wait blocks only the waiting task.
begin
    sleep 0.1
    echo awaited
end &
begin
    sleep 0.02
    echo other
end &
wait
# CHECK: other
# CHECK: awaited

# A job started by a task outlives that task and is collected by wait.
begin
    sleep 0.2 &
end &
sleep 0.05
set -l job_pids (jobs -p)
string match -rq '^[0-9]+$' $job_pids
echo child_job=$status
wait
# CHECK: child_job=0

# A task has a copy of the store: cd and set -g change only the task.
set -l before (pwd)
begin
    cd /
    set -g task_variable inside
end &
wait
test (pwd) = "$before"
echo cwd_unchanged=$status
set -q task_variable
echo variable_isolated=$status
# CHECK: cwd_unchanged=0
# CHECK: variable_isolated=1

# Universal variables are shared by every task.
begin
    set -U task_universal shared
end &
wait
echo universal=$task_universal
set -Ue task_universal
# CHECK: universal=shared

# Two task jobs interleave on their child processes: the earlier one writes the winner.
function race_winner
    begin
        sleep 0.1
        printf 'quick\n' >$argv[1]
    end &
    begin
        sleep 0.2
        test -e $argv[1]; and return
        printf 'slow\n' >$argv[1]
    end &
    wait
end
set -l winner (mktemp)
race_winner $winner
cat $winner
command rm -f $winner
# CHECK: quick

# & emits no value and the job is listed while it runs.
set -l words (sleep 0.1 &)
count $words
# CHECK: 0
set -l listing (jobs)
string match -q '*sleep 0.1*' $listing
echo job_listed=$status
wait
# CHECK: job_listed=0

# wait --emit writes an identity/status pair for each observed child.
sleep 0.2 &
sleep 0.1 &
wait --emit sleep
# CHECK: {{^\d+$}}
# CHECK: 0
# CHECK: {{^\d+$}}
# CHECK: 0

# wait --emit with no operands observes every waitable background job.
sleep 0.2 &
sleep 0.1 &
wait --emit
# CHECK: {{^%\d+$}}
# CHECK: 0
# CHECK: {{^%\d+$}}
# CHECK: 0

# --emit is long-only.
wait -e 2>/dev/null
echo short_emit=$status
wait --emit
echo long_emit=$status
# CHECK: short_emit=2
# CHECK: long_emit=0
