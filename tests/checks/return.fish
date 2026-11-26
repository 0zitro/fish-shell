#RUN: fish=%fish %fish %s
# Some tests of the "return" builtin.

$fish -c 'return 5'
echo $status
# CHECK: 5

$fish -c 'exit 5'
echo $status
# CHECK: 5

$fish -c 'echo foo; return 2; echo bar'
# CHECK: foo
# but not bar
echo $status
# CHECK: 2

begin
    $fish -ic 'echo interactive-foo; return 69; echo interactive-bar'
    # CHECK: interactive-foo
    # but not bar
    echo $status
    # CHECK: 69
end

# Verify negative return values don't cause UB and never map to 0
function empty_return
    return $argv[1]
end

for i in (seq -- -550 -1)
    empty_return $i
    if test $status -eq 0
        echo returning $i from a fish script maps to a $status of 0!
    end
end
# CHECK:

$fish -c "return 1 2"
# CHECKERR: return: too many arguments
# CHECKERR: Standard input (line 1):
# CHECKERR: {{^}}return 1 2
# CHECKERR: {{^}}^
# CHECKERR: (Type 'help return' for related documentation)

$fish -c "return abc"
# CHECKERR: return: abc: invalid integer
# CHECKERR: Standard input (line 1):
# CHECKERR: {{^}}return abc
# CHECKERR: {{^}}^
# CHECKERR: (Type 'help return' for related documentation)

# In a function, return exits the function and not its caller.
function return_from_function
    echo in-function
    return 7
    echo unreachable
end
return_from_function
# CHECK: in-function
echo $status
# CHECK: 7

# In a command substitution, return ends the substitution, keeps its output,
# and the enclosing function continues.
function return_in_substitution
    set -l value (echo inner; return 7; echo unreachable)
    echo value=$value
end
return_in_substitution
# CHECK: value=inner

# A pipeline stage's return sets the stage status and does not end the
# enclosing function.
function return_in_pipeline
    echo pre
    true | return 7
    echo post status=$status
end
return_in_pipeline
# CHECK: pre
# CHECK: post status=7

# Return in a pipeline at the top level does not exit the script.
$fish -c 'echo before; true | return 7; echo after status=$status'
# CHECK: before
# CHECK: after status=7

# A variable-assignment substitution is validated like an argument: the
# accepted return chain reproduces the captured statuses, and the rejected
# pipeline command is reported.
$fish -c 'value=(echo start; true | return 7) echo value=$value status=$status count=(count $pipestatus)'
# CHECK: value=start status=7 count=2
$fish -c 'value=(break | true) echo after'
# CHECKERR: fish: The 'break' command can not be used in a pipeline
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}
