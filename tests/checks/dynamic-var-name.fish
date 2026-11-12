# RUN: fish=%fish %fish %s

set n x
set x 123
echo $$(echo n)
# CHECK: x

echo "$$(echo n)"
# CHECK: x

echo pre"$$(echo n)"post
# CHECK: prexpost

# An escaped dollar is literal and does not join a dereference run.
echo \$$(echo n)
# CHECK: $n
echo "\$$(echo n)"
# CHECK: $n
echo x\$$(echo n)
# CHECK: x$n
echo \\$$(echo n)
# CHECK: \x

# Following source is a suffix on the produced reference, not part of its name.
echo $$(echo n)post
# CHECK: xpost
echo "$$(echo n)post"
# CHECK: xpost
echo pre$$(echo n)post
# CHECK: prexpost
echo $$$(echo n)2
# CHECK: 1232

echo $$(printf 'n\n')
# CHECK: x

set n cmd
set cmd echo
$$(echo cmd) hello
# CHECK: hello

set n x
set x 123
echo $$n
# CHECK: 123

set x y
set y z
set z 123
echo $$(echo x)
# CHECK: y
echo $$$(echo x)
# CHECK: z
echo $$$$(echo x)
# CHECK: 123

echo $$(echo $$(echo x))
# CHECK: z

echo "$$(echo $$(echo x))"
# CHECK: z

echo "$$$(echo x)"
# CHECK: z

set x a b c
echo $$(printf 'x[1]\n')
# CHECK: a
echo $$(printf 'x[2..3]\n')
# CHECK: b c

set a A
echo $$$(printf 'x[1]\n')
# CHECK: A

echo $$(printf 'x[1 ]\n')
# CHECK: a

$fish -c 'set n x; set x 123; echo $$(printf "")'
# CHECKERR: fish: Dynamic variable reference must be exactly one variable reference
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

$fish -c 'set n x; set x 123; echo $$(printf "n\nx\n")'
# CHECKERR: fish: Dynamic variable reference must be exactly one variable reference
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

$fish -c 'set x a b c; echo $$(printf "x[1][2]\n")'
# CHECKERR: fish: Dynamic variable reference must be exactly one variable reference
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

$fish -c 'set x a b c; echo $$(printf "x[-1]\n")'
# CHECK: c

$fish -c 'echo $$'
# CHECKERR: fish: $$ is not the pid. In fish, please use $fish_pid.
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}
