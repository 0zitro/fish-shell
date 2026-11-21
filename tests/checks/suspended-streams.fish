# RUN: fish=%fish %fish %s

# The command is a builtin: the shell records the stream itself.
builtin -q suspend
echo $status
# CHECK: 0

# Malformed options are rejected before any stream is recorded.
suspend --no-such-option 2>/dev/null
echo $status
# CHECK: 2

# A redirected standard input becomes a suspension, whose handle is the command's output.
set -l handle (printf 'one\ntwo\n' | suspend)
string match -qr '^\^[0-9]+$' -- $handle
echo handle_shape=$status
# CHECK: handle_shape=0
echo handle_count=(count $handle)
# CHECK: handle_count=1

# A stage after suspend receives the handle like any other output.
set -l middle (printf 'data\n' | cat --show-all | suspend | command cat --show-all)
string match -qr '^\^[0-9]+\$$' -- $middle
echo middle_handle=$status
# CHECK: middle_handle=0

# The command does not block: the suspended job keeps running in the background.
set -l running_handle (sleep 0.3 | suspend)
jobs -q
echo running_job=$status
# CHECK: running_job=0
sleep 0.5
jobs -q
echo finished_job=$status
# CHECK: finished_job=1

# A producer larger than the pipe buffer does not block the suspending statement.
set -l large_handle (seq 20000 | suspend)
string match -qr '^\^[0-9]+$' -- $large_handle
echo large_handle=$status
# CHECK: large_handle=0

# resume writes the suspended stream to its own output.
set -l first_handle (echo 1 | cat --show-all | suspend)
resume $first_handle
# CHECK: 1$

# A stage after resume reads the stream directly.
set -l second_handle (echo 1 | cat --show-all | suspend | command cat)
resume $second_handle | command cat --show-all
# CHECK: 1$$

# The stream reaches a command substitution.
set -l capture_handle (printf 'one\ntwo\n' | suspend)
set -l captured (resume $capture_handle)
echo captured=(string join , -- $captured)
# CHECK: captured=one,two

# resume waits for a stream larger than the pipe buffer.
set -l large_resume_handle (seq 20000 | suspend)
set -l line_count (resume $large_resume_handle | command wc -l)
echo lines=(string trim -- $line_count)
# CHECK: lines=20000

# Nested fd-pipe stages suspend two descriptors of one pipeline.
# The `1>| suspend` must be the inner stage: a suspend writes its handle to the
# fd 1 of its enclosing context, so an outer `1>| suspend` would record the
# inner handle instead of the stream. Each suspend writes its own handle to its
# own redirection, so the two streams are recovered independently.
set -l stdout_handle_file (mktemp)
set -l stderr_handle_file (mktemp)
begin
    begin
        echo stdout >&1
        echo stderr >&2
    end 1>| suspend >$stdout_handle_file
end 2>| suspend >$stderr_handle_file
set -l stdout_handle (cat $stdout_handle_file)
set -l stderr_handle (cat $stderr_handle_file)
rm -f $stdout_handle_file $stderr_handle_file
string match -qr '^\^[0-9]+$' -- $stdout_handle $stderr_handle
echo split_handles=$status
# CHECK: split_handles=0
resume $stdout_handle
# CHECK: stdout
resume $stderr_handle
# CHECK: stderr

# A handle denotes one suspension.
$fish -c 'set -l h (printf x | suspend); resume $h >/dev/null; resume $h' 2>/dev/null
echo consumed=$status
# CHECK: consumed=1

# A failed resume leaves the handle usable.
set -l retry_handle (printf 'retry\n' | suspend)
resume $retry_handle extra 2>/dev/null
echo usage_status=$status
# CHECK: usage_status=2
resume $retry_handle
# CHECK: retry

# A malformed or unknown handle is rejected.
$fish -c 'resume ^999' 2>/dev/null
echo unknown=$status
# CHECK: unknown=1
$fish -c 'resume not-a-handle' 2>/dev/null
echo malformed=$status
# CHECK: malformed=2
$fish -c 'resume' 2>/dev/null
echo missing_handle=$status
# CHECK: missing_handle=2
