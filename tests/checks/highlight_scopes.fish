# RUN: fish_indent=%fish_indent %fish %s

# The highlighting pass gives every run of characters one scope, and this test states
# the scope of each run a snippet is classified into: the character a run starts at,
# then its path.  The command line, the pager rows and `fish_indent` all take their
# tokens from this pass.  A run also states the scopes that enclose it and the
# attributes it carries; this test reads the path.

function runs_of
    $fish_indent --pygments | string split -- '\n' | string replace --regex -- '^(\d+),\d+,([^,]*),.*$' '$1 $2'
end

printf '%s' 'set -gx name 1' | runs_of
# CHECK: 0 command.builtin.set
# CHECK: 3 source
# CHECK: 4 argument.option.gnu
# CHECK: 7 source
# CHECK: 8 variable.definition.global.exported
# CHECK: 12 source
# CHECK: 13 argument

printf '%s' 'echo --long -5' | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 argument.option.long
# CHECK: 11 source
# CHECK: 12 argument.option.numeric

printf '%s' 'set --path P ~/x' | runs_of
# CHECK: 0 command.builtin.set
# CHECK: 3 source
# CHECK: 4 argument.option.long
# CHECK: 10 source
# CHECK: 11 variable.definition.path
# CHECK: 12 source
# CHECK: 13 operator.tilde
# CHECK: 14 argument

printf '%s' 'echo hi > /tmp/out' | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 argument
# CHECK: 7 source
# CHECK: 8 redirection.output
# CHECK: 9 source
# CHECK: 10 redirection.output.target

# A redirection's target is a word: its substitution, its quotes and its escapes keep
# their own scopes, and the scope of the target is what the text no piece names takes.
printf '%s' 'echo hi >>"$(true)"__\'z\'' | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 argument
# CHECK: 7 source
# CHECK: 8 redirection.append
# CHECK: 10 string.quoted.double.delimiter
# CHECK: 11 operator.dollar
# CHECK: 12 operator.parenthesis
# CHECK: 13 command
# CHECK: 17 operator.parenthesis
# CHECK: 18 string.quoted.double.delimiter
# CHECK: 19 redirection.append.target
# CHECK: 21 string.quoted.single.delimiter
# CHECK: 22 string.quoted.single
# CHECK: 23 string.quoted.single.delimiter

printf '%s' 'echo hi | cat' | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 argument
# CHECK: 7 source
# CHECK: 8 terminator.pipe
# CHECK: 9 source
# CHECK: 10 command

printf '%s' 'echo *' | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 operator.glob

printf '%s' 'if true; end' | runs_of
# CHECK: 0 keyword.if
# CHECK: 2 source
# CHECK: 3 command
# CHECK: 7 terminator.semicolon
# CHECK: 8 source
# CHECK: 9 keyword.end

printf '%s' "echo 'unterminated" | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 invalid
# CHECK: 6 string.quoted.single
