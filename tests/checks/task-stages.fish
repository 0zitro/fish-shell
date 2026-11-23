# RUN: %fish %s

# A stage of a background job whose command the shell evaluates is a task and has no process
# identifier; jobs -p prints only the process stages.
begin
    echo task
end | sleep 0.2 &
set -l job_pids (jobs -p)
count $job_pids
# CHECK: 1
wait

# A pipeline of process stages keeps one identifier per stage.
sleep 0.2 | sleep 0.2 &
set -l process_pids (jobs -p)
count $process_pids
# CHECK: 2
wait

# `exit N` in a task finishes the task; the shell continues.
begin
    exit 3
end &
echo shell continues
# CHECK: shell continues
wait
