# RUN: %fish %s

# The command is a builtin: job construction resolves the suspending forms from it.
builtin -q suspend
echo $status
# CHECK: 0

# Options are rejected before the shell suspends itself.
suspend --no-such-option 2>/dev/null
echo $status
# CHECK: 2
