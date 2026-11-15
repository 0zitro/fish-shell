# RUN: fish_indent=%fish_indent %fish %s

# The highlighting pass gives every run of characters one scope, and this test states
# the scope of each run a snippet is classified into: the character a run starts at,
# then its path.  The command line, the pager rows and `fish_indent` all take their
# tokens from this pass.  A run also states the scopes that enclose it and the
# attributes it carries; this test reads the path.

function runs_of
    $fish_indent --pygments | string split -- '\n' | string replace --regex -- '^(\d+),\d+,([^,]*),.*$' '$1 $2'
end

function enclosing_of
    $fish_indent --pygments | string split -- '\n' \
        | string replace --regex -- '^(\d+),[^,]*,[^,]*,([^,]*),.*$' '$1 $2' \
        | string trim --right
end

printf '%s' 'set -gx name 1' | runs_of
# CHECK: 0 command.builtin.set
# CHECK: 3 source
# CHECK: 4 argument.option.gnu
# CHECK: 7 source
# CHECK: 8 variable.definition.global.exported
# CHECK: 12 source
# CHECK: 13 argument.number

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

# A tilde expands at a word's start only: an option's value is inside the option's word,
# so the tilde there is the character it was written as.
printf '%s' 'echo --path=~/x' | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 argument.option.long
# CHECK: 11 operator.assignment
# CHECK: 12 argument

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
# CHECK: 10 string.delimiter.double
# CHECK: 11 operator.dollar
# CHECK: 12 operator.parenthesis
# CHECK: 13 command
# CHECK: 17 operator.parenthesis
# CHECK: 18 string.delimiter.double
# CHECK: 19 redirection.append.target
# CHECK: 21 string.delimiter.single
# CHECK: 22 meta.string.quoted.single
# CHECK: 23 string.delimiter.single

# A bare number and a bare boolean are arguments that narrow; the quoted forms are quotes.
printf '%s' 'test "$x" -gt 123' | runs_of
# CHECK: 0 command.builtin.test
# CHECK: 4 source
# CHECK: 5 string.delimiter.double
# CHECK: 6 variable.sigil
# CHECK: 7 variable
# CHECK: 8 string.delimiter.double
# CHECK: 9 source
# CHECK: 10 argument.option.gnu
# CHECK: 13 source
# CHECK: 14 argument.number

printf '%s' 'echo '\''123'\'' "true"' | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 string.delimiter.single
# CHECK: 6 meta.string.quoted.single
# CHECK: 9 string.delimiter.single
# CHECK: 10 source
# CHECK: 11 string.delimiter.double
# CHECK: 12 meta.string.quoted.double
# CHECK: 16 string.delimiter.double

# A slice's bounds are words of the slice: a signed number names a position.
printf '%s' 'echo $x[-1..2]' | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 variable.sigil
# CHECK: 6 variable
# CHECK: 7 variable.slice
# CHECK: 8 argument.number
# CHECK: 10 variable.slice
# CHECK: 12 argument.number
# CHECK: 13 variable.slice

printf '%s' 'set --local -- guard false' | runs_of
# CHECK: 0 command.builtin.set
# CHECK: 3 source
# CHECK: 4 argument.option.long
# CHECK: 11 source
# CHECK: 12 argument.option
# CHECK: 14 source
# CHECK: 15 variable.definition.local
# CHECK: 20 source
# CHECK: 21 argument.boolean

# A long option's value is an argument of its own, so a literal there narrows too.
printf '%s' 'echo --opt=1' | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 argument.option.long
# CHECK: 10 operator.assignment
# CHECK: 11 argument.number

# So is the value an assignment gives the name it defines.
printf '%s' 'dup=2 echo' | runs_of
# CHECK: 0 variable.definition.local.exported
# CHECK: 3 operator.assignment
# CHECK: 4 argument.number
# CHECK: 5 source
# CHECK: 6 command

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

# A begin header's options are the header's own words, and the whitespace around them is
# whitespace.
printf '%s' 'begin -qq echo hi; end' | runs_of
# CHECK: 0 keyword.begin
# CHECK: 5 source
# CHECK: 6 argument.option.gnu
# CHECK: 9 source
# CHECK: 10 command
# CHECK: 14 source
# CHECK: 15 argument
# CHECK: 17 terminator.semicolon
# CHECK: 18 source
# CHECK: 19 keyword.end

printf '%s' 'begin --quoted --quoted echo hi; end' | runs_of
# CHECK: 0 keyword.begin
# CHECK: 5 source
# CHECK: 6 argument.option.long
# CHECK: 14 source
# CHECK: 15 argument.option.long
# CHECK: 23 source
# CHECK: 24 command
# CHECK: 28 source
# CHECK: 29 argument
# CHECK: 31 terminator.semicolon
# CHECK: 32 source
# CHECK: 33 keyword.end

# A barrier option is a header option too.
printf '%s' 'begin -b echo hi; end' | runs_of
# CHECK: 0 keyword.begin
# CHECK: 5 source
# CHECK: 6 argument.option.short
# CHECK: 8 source
# CHECK: 9 command
# CHECK: 13 source
# CHECK: 14 argument
# CHECK: 16 terminator.semicolon
# CHECK: 17 source
# CHECK: 18 keyword.end

# A slice encloses what stands between its brackets, and the brackets themselves state no
# enclosing scope.
printf '%s' 'echo $x[$i]' | enclosing_of
# CHECK: 0
# CHECK: 4
# CHECK: 5
# CHECK: 6
# CHECK: 7
# CHECK: 8 meta.slice
# CHECK: 9 meta.slice
# CHECK: 10

# A brace clause is a block of its own: its marks name the pair, and its body is a block
# and a brace clause in particular, where a begin block's body is only a block.
printf '%s' '{ echo hi; }' | runs_of
# CHECK: 0 keyword.brace.open
# CHECK: 1 source
# CHECK: 2 command
# CHECK: 6 source
# CHECK: 7 argument
# CHECK: 9 terminator.semicolon
# CHECK: 10 source
# CHECK: 11 keyword.brace.close

printf '%s' '{ echo hi; }' | enclosing_of
# CHECK: 0
# CHECK: 1
# CHECK: 2 meta.block+meta.block.brace
# CHECK: 6
# CHECK: 7 meta.block+meta.block.brace
# CHECK: 9 meta.block+meta.block.brace
# CHECK: 10
# CHECK: 11

printf '%s' 'begin; echo; end' | enclosing_of
# CHECK: 0
# CHECK: 5
# CHECK: 6
# CHECK: 7 meta.block
# CHECK: 11 meta.block
# CHECK: 12
# CHECK: 13

# A line continuation escapes the newline it ends, and the indentation after it is the
# whitespace it is.
printf 'echo a \\\n    b' | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 argument
# CHECK: 6 source
# CHECK: 7 string.escape
# CHECK: 9 source
# CHECK: 13 argument

printf '%s' 'if true; end' | runs_of
# CHECK: 0 keyword.if
# CHECK: 2 source
# CHECK: 3 command
# CHECK: 7 terminator.semicolon
# CHECK: 8 source
# CHECK: 9 keyword.end

# A definition's name is the command it declares: the header states that kind, so the name
# is named without resolving anything, and the flag the body's enclosure states is stated
# on the name too.
printf '%s' 'function f; echo; end' | runs_of
# CHECK: 0 keyword.function
# CHECK: 8 source
# CHECK: 9 command.function.definition
# CHECK: 10 terminator.semicolon
# CHECK: 11 source
# CHECK: 12 command
# CHECK: 16 terminator.semicolon
# CHECK: 17 source
# CHECK: 18 keyword.end

printf '%s' 'function g -S; end' | runs_of
# CHECK: 0 keyword.function
# CHECK: 8 source
# CHECK: 9 command.function.definition.no_shadowing
# CHECK: 10 source
# CHECK: 11 argument.option.short
# CHECK: 13 terminator.semicolon
# CHECK: 14 source
# CHECK: 15 keyword.end

# A quoted run is an enclosing construct: its own characters are its scope, and what it
# expands — a reference, a substitution, an escape, its marks — is stated inside it.
printf '%s' 'echo "$(true)"' | enclosing_of
# CHECK: 0
# CHECK: 4
# CHECK: 5 meta.string.quoted+meta.string.quoted.double
# CHECK: 6 meta.string.quoted+meta.string.quoted.double
# CHECK: 7 meta.string.quoted+meta.string.quoted.double
# CHECK: 8 meta.string.quoted+meta.string.quoted.double+meta.command_substitution
# CHECK: 12 meta.string.quoted+meta.string.quoted.double
# CHECK: 13 meta.string.quoted+meta.string.quoted.double

printf '%s' "echo 'ab'" | enclosing_of
# CHECK: 0
# CHECK: 4
# CHECK: 5 meta.string.quoted+meta.string.quoted.single
# CHECK: 6
# CHECK: 8 meta.string.quoted+meta.string.quoted.single

printf '%s' "echo 'unterminated" | runs_of
# CHECK: 0 command
# CHECK: 4 source
# CHECK: 5 invalid
# CHECK: 6 meta.string.quoted.single
