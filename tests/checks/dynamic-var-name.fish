# RUN: fish=%fish %fish %s

set n x
set x 123
echo $$(echo n)
# CHECK: 123

echo "$$(echo n)"
# CHECK: 123

echo pre"$$(echo n)"post
# CHECK: pre123post

echo $$(printf 'n\n')
# CHECK: 123

set n cmd
set cmd echo
$$(echo n) hello
# CHECK: hello

set n x
echo $$n
# CHECK: 123

$fish -c 'set n x; set x 123; echo $$(printf "")'
# CHECKERR: fish: Dynamic variable name must be a single valid variable name
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

$fish -c 'set n x; set x 123; echo $$(printf "n\nx\n")'
# CHECKERR: fish: Dynamic variable name must be a single valid variable name
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

$fish -c 'echo $$'
# CHECKERR: fish: $$ is not the pid. In fish, please use $fish_pid.
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}
