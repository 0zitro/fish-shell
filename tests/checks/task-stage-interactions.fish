# RUN: %fish %s

# A stage task may block and resume more than once.
begin
    set -l a (/bin/echo y)
    set -l b (/bin/echo z)
    echo a=$a b=$b
end &
builtin wait
# CHECK: a=y b=z

begin
    cat (echo from-task | psub)
end &
builtin wait
# CHECK: from-task

# A builtin stage of a background job is a task, so an enclosing capture does not
# receive its output.
set -l captured (echo hi &)
builtin wait
echo "captured=$captured"
# CHECK: hi
# CHECK: captured=

# exec in a task starts the command as a process stage and discards the task's
# evaluation.
begin
    echo before-exec
    exec /bin/echo exec-ran
    echo unreachable-exec
end &
builtin wait
echo after-exec=$status
# CHECK: before-exec
# CHECK: exec-ran
# CHECK: after-exec=0

# A handler registered by a task is released with the task.
begin
    function task_handler --on-event task_event
        echo task-handler-ran
    end
end &
builtin wait
set -l first_emit (emit task_event 2>&1)
echo "first_emit=[$first_emit]"
# CHECK: first_emit=[]
set -l second_emit (emit task_event 2>&1)
echo "second_emit=[$second_emit]"
# CHECK: second_emit=[]

# Waiting on the calling task's own job is refused rather than blocked.
begin
    builtin wait %1
    echo task-done
end &
/bin/sleep 0.2
echo session-survived
# CHECK: task-done
# CHECK: session-survived
# CHECKERR: wait: %1: the calling task's own job

# A bare return in a background task ends the task, not the session script.
begin
    return 7
    echo unreachable-return
end &
builtin wait
echo after-return=$status
# CHECK: after-return=7

# jobs -c reports a task stage without a process argument to quote.
begin
    /bin/sleep 0.2
end &
jobs -c %1
# CHECK: begin
# CHECK: /bin/sleep 0.2
# CHECK: end &
builtin wait

# A background task with no later blocking point runs before the script ends.
begin
    echo task-output-at-exit
end &
echo script-end
# CHECK: script-end
# CHECK: task-output-at-exit
