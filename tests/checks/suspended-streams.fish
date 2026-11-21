# RUN: %fish %s

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
