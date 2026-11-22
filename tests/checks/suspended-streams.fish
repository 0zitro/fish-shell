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

# The command form starts its source as one pipeline with the standard descriptors suspended.
set -l command_handle (suspend echo command)
resume $command_handle
# CHECK: command

# Its standard error is suspended as one stream.
set -l error_file (mktemp)
set -l error_handle (suspend command ls /definitely-not-here)
resume $error_handle >/dev/null 2>$error_file
test -s $error_file
echo command_stderr=$status
# CHECK: command_stderr=0
rm -f $error_file

# Its standard input comes from the resuming command's standard input.
set -l feed_handle (suspend cat)
echo fed | resume $feed_handle
# CHECK: fed

# A bound output descriptor is one spliced stream across the stages of the source.
set -l output_handle (suspend --fd 5 'echo one >&5 | echo two >&5')
resume $output_handle 5>| command sort
# CHECK: one
# CHECK: two

# A bound input descriptor reads from the descriptor the resuming command opens.
set -l input_file (mktemp)
echo hello >$input_file
set -l input_handle (suspend --fd 5:in 'cat <&5')
resume $input_handle 5<$input_file
# CHECK: hello
rm -f $input_file

# A binding defaults to output; the standard descriptors and unknown directions are rejected.
$fish -c 'suspend --fd 1 echo x' 2>/dev/null
echo bind_standard=$status
# CHECK: bind_standard=2
$fish -c 'suspend --fd 5:x echo x' 2>/dev/null
echo bind_direction=$status
# CHECK: bind_direction=2
$fish -c 'suspend --fd 5 --fd 5 echo x' 2>/dev/null
echo bind_duplicate=$status
# CHECK: bind_duplicate=2
