#RUN: %fish %s
# Test the `functions` builtin

function f1
end

# ==========
# Verify that `functions --details` works as expected when given too many args.
functions    --details f1 f2
#CHECKERR: functions: --details: expected 1 arguments; got 2

# Verify that it still mentions "--details" even if it isn't the last option.
functions    --details --verbose f1 f2
#CHECKERR: functions: --details: expected 1 arguments; got 2

# ==========
# Verify that `functions --details` works as expected when given the name of a
# known function.
functions --details f1
#CHECK: {{.*}}checks/functions.fish

# ==========
# Verify that `functions --details` works as expected when given the name of an
# unknown function.
functions -D f2
#CHECK: n/a

# ==========
# Verify that `functions --details` works as expected when given the name of a
# function that could be autoloaded but isn't currently loaded.
set x (functions -D vared)
if test (count $x) -ne 1
    or not string match -rq '.*functions/vared\.fish' "$x"
    echo "Unexpected output for 'functions -D vared': $x" >&2
end

# ==========
# Verify that `functions --verbose --details` works as expected when given the name of a
# function that was autoloaded.
set x (functions -v -D vared)
if test (count $x) -ne 5
    or not string match -rq '.*functions/vared\.fish' $x[1]
    or test $x[2] != autoloaded
    or test $x[3] != 7
    or test $x[4] != scope-shadowing
    or test $x[5] != 'Edit variable value'
    echo "Unexpected output for 'functions -v -D vared': $x" >&2
end

# ==========
# Verify that `functions --verbose --details` properly escapes a function
# with a multiline description.
function multiline_descr -d 'line 1\n
line 2 & more; way more'
end
set x (functions -v -D multiline_descr)
if test $x[5] != 'line 1\\\\n\\nline 2 & more; way more'
    echo "Unexpected output for 'functions -v -D multiline_descr': $x" >&2
end

# ==========
# Verify that `functions --details` works as expected when given the name of a
# function that is copied. (Prints the filename where it was copied.)
functions -c f1 f1a
functions -D f1a
#CHECK: {{.*}}checks/functions.fish
functions -Dv f1a
#CHECK: {{.*}}checks/functions.fish
#CHECK: {{.*}}checks/functions.fish
#CHECK: {{\d+}}
#CHECK: scope-shadowing
#CHECK:
echo "functions -c f1 f1b" | source
functions -D f1b
#CHECK: -
functions -Dv f1b
#CHECK: -
#CHECK: {{.*}}checks/functions.fish
#CHECK: {{\d+}}
#CHECK: scope-shadowing
#CHECK:

# ==========
# Verify function description setting
function test_func_desc
end
functions test_func_desc | string match --quiet '*description*'
and echo "Unexpected description" >&2

functions --description description1 test_func_desc
functions test_func_desc | string match --quiet '*description1*'
or echo "Failed to find description 1" >&2

functions -d description2 test_func_desc
functions test_func_desc | string match --quiet '*description2*'
or echo "Failed to find description 2" >&2

# ==========
# Verify that the functions are printed in order.
functions f1 test_func_desc
# CHECK: # Defined in {{.*}}
# CHECK: function f1
# CHECK: end
# CHECK: # Defined in {{.*}}
# CHECK: function test_func_desc --description description2
# CHECK: end

# Note: This test isn't ideal - if ls was loaded before,
# or doesn't exist, it'll succeed anyway.
#
# But we can't *confirm* that an ls function exists,
# so this is the best we can do.
functions --erase ls
type -t ls
#CHECK: file

# ==========
# Verify that `functions --query` does not return 0 if there are 256 missing functions
functions --query a(seq 1 256)
echo $status
#CHECK: 255

echo "function t; echo tttt; end" | source
functions t
# CHECK: # Defined via `source`
# CHECK: function t
# CHECK: echo tttt;
# CHECK: end

functions --no-details t
# CHECK: function t
# CHECK: echo tttt;
# CHECK: end

functions -c t t2
functions t2
# CHECK: # Defined via `source`, copied in {{.*}}checks/functions.fish @ line {{\d+}}
# CHECK: function t2
# CHECK: echo tttt;
# CHECK: end
functions -D t2
#CHECK: {{.*}}checks/functions.fish
functions -Dv t2
#CHECK: {{.*}}checks/functions.fish
#CHECK: -
#CHECK: {{\d+}}
#CHECK: scope-shadowing
#CHECK:

echo "functions -c t t3" | source
functions t3
# CHECK: # Defined via `source`, copied via `source`
# CHECK: function t3
# CHECK: echo tttt;
# CHECK: end
functions -D t3
#CHECK: -
functions -Dv t3
#CHECK: -
#CHECK: -
#CHECK: {{\d+}}
#CHECK: scope-shadowing
#CHECK:

functions --no-details t2
# CHECK: function t2
# CHECK: echo tttt;
# CHECK: end

functions --no-details --details t
# CHECKERR: functions: invalid option combination
# CHECKERR:
# CHECKERR: {{.*}}checks/functions.fish (line {{\d+}}):
# CHECKERR: functions --no-details --details t
# CHECKERR: ^
# CHECKERR: (Type 'help functions' for related documentation)
# XXX FIXME ^ caret should point at --no-details --details

function term1 --on-signal TERM
end
function term2 --on-signal TERM
end
function term3 --on-signal TERM
end

functions --handlers-type signal
# CHECK: Event signal
# CHECK: SIGTRAP fish_sigtrap_handler
# CHECK: SIGTERM term1
# CHECK: SIGTERM term2
# CHECK: SIGTERM term3

# See how --names and --all work.
# We don't want to list all of our functions here,
# so we just match a few that we know are there.
functions -n | string match cd
# CHECK: cd

functions --names | string match __fish_config_interactive
echo $status
# CHECK: 1

functions --names -a | string match __fish_config_interactive
# CHECK: __fish_config_interactive

functions --description ""
# CHECKERR: functions: Expected exactly one function name
# CHECKERR: {{.*}}checks/functions.fish (line {{\d+}}):
# CHECKERR: functions --description ""
# CHECKERR: ^
# CHECKERR: (Type 'help functions' for related documentation)

function foo --on-variable foo; end
# This should print *everything*
functions --handlers-type "" | string match 'Event *'
# CHECK: Event signal
# CHECK: Event variable
# CHECK: Event generic
functions -e foo

functions --details --verbose thisfunctiondoesnotexist
# CHECK: n/a
# CHECK: n/a
# CHECK: 0
# CHECK: n/a
# CHECK: n/a

functions --banana
# CHECKERR: functions: --banana: unknown option
echo $status
# CHECK: 2

functions --all=arg
# CHECKERR: functions: --all=arg: option does not take an argument
echo $status
# CHECK: 2

# Test --color option
function test_color_option
    echo hello
end

function transparent_details --no-scope-shadowing=transparent
end
functions -Dv transparent_details
#CHECK: {{.*}}checks/functions.fish
#CHECK: not-autoloaded
#CHECK: {{\d+}}
#CHECK: transparent
#CHECK:

functions --color=invalid
# CHECKERR: functions: Invalid value for '--color' option: 'invalid'. Expected 'always', 'never', or 'auto'

functions --no-details --color=never test_color_option
# CHECK: function test_color_option
# CHECK:     echo hello
# CHECK: end

string escape (functions --no-details --color=always test_color_option)
# CHECK: function\ \e\[36mtest_color_option\e\[32m
# CHECK: \e\[m\ \ \ \ echo\ \e\[36mhello\e\[32m
# CHECK: \e\[mend\e\[32m\e\[m

# Test --outer for top-level definition.
function no_outer_target
end
functions --outer no_outer_target
# CHECKERR: functions: Function 'no_outer_target' has no outer function
echo $status
# CHECK: 1

# Test --outer with an available outer function.
function outer_alive
    function inner_alive
    end
end
outer_alive
functions --outer inner_alive
# CHECK: outer_alive
echo $status
# CHECK: 0
functions --outer=current inner_alive
# CHECK: outer_alive
echo $status
# CHECK: 0

# Test --outer when the captured outer function is no longer available.
function outer_dead
    function inner_dead
    end
end
outer_dead
functions -e outer_dead
functions --outer inner_dead
# CHECKERR: functions: Outer function 'outer_dead' for 'inner_dead' is no longer available
echo $status
# CHECK: 3

# Test --outer=initial for nested definitions.
function outer_initial
    function middle_initial
        function inner_initial
        end
    end
    middle_initial
end
outer_initial
functions --outer=initial inner_initial
# CHECK: middle_initial
echo $status
# CHECK: 0

# Test --outer=initial keeps the first anchor across same-name redefinition.
function outer_initial_reset_a
    function inner_initial_reset
    end
end
outer_initial_reset_a
functions --outer=initial inner_initial_reset
# CHECK: outer_initial_reset_a
echo $status
# CHECK: 0
function outer_initial_reset_b
    function inner_initial_reset
    end
end
outer_initial_reset_b
functions --outer=initial inner_initial_reset
# CHECK: outer_initial_reset_a
echo $status
# CHECK: 0

# Test --outer=initial copies preserve source lineage anchor, not copy call-site.
function outer_initial_copy_anchor
    function inner_initial_copy_source
    end
end
outer_initial_copy_anchor
function outer_initial_copy_wrapper
    functions -c inner_initial_copy_source inner_initial_copy_target
end
outer_initial_copy_wrapper
functions -e outer_initial_copy_wrapper
functions --outer=initial inner_initial_copy_target
# CHECK: outer_initial_copy_anchor
echo $status
# CHECK: 0

# Test that same-name redefinition invalidates old generations.
function outer_same_name
    function inner_same_name
    end
end
outer_same_name
functions -c inner_same_name inner_same_name_old
echo "function outer_same_name; function inner_same_name; end; end" | source
outer_same_name
functions --outer inner_same_name
# CHECK: outer_same_name
echo $status
# CHECK: 0
functions --outer inner_same_name_old
# CHECKERR: functions: Outer function 'outer_same_name' for 'inner_same_name_old' is no longer available
echo $status
# CHECK: 3

# Test copy/redefine scenario: latest inner points to copied outer, copied old inner points to original outer.
function outer_copy_source
    function inner_copy_target
    end
end
outer_copy_source
functions -c outer_copy_source outer_copy_new
functions -c inner_copy_target inner_copy_old
outer_copy_new
functions --outer inner_copy_target
# CHECK: outer_copy_new
echo $status
# CHECK: 0
functions --outer inner_copy_old
# CHECK: outer_copy_source
echo $status
# CHECK: 0

# Test dynamic naming with redefinition churn.
for i in a builtin
    function outer__dynamic__$i
        function xyz
        end
    end
    outer__dynamic__$i
end
functions --outer xyz
# CHECK: outer__dynamic__builtin
echo $status
# CHECK: 0

# Test no-autoload behavior for --outer.
set -l old_fish_function_path $fish_function_path
set -l autoload_probe_dir (mktemp -d)
printf 'set -g __outer_autoload_marker loaded\nfunction __outer_autoload_probe\nend\n' >$autoload_probe_dir/__outer_autoload_probe.fish
set -gx fish_function_path $autoload_probe_dir $fish_function_path
set -e __outer_autoload_marker
functions --outer __outer_autoload_probe
# CHECKERR: functions: Function '__outer_autoload_probe' is not currently available
echo $status
# CHECK: 5
set -q __outer_autoload_marker
echo $status
# CHECK: 1
set -gx fish_function_path $old_fish_function_path

# Test --outer argument and mode validation.
functions --outer
# CHECKERR: functions: --outer: expected 1 arguments; got 0
echo $status
# CHECK: 2
functions --outer f1 f2
# CHECKERR: functions: --outer: expected 1 arguments; got 2
echo $status
# CHECK: 2
functions --outer --query f1
# CHECKERR: functions: invalid option combination
# CHECKERR:
# CHECKERR: {{.*}}checks/functions.fish (line {{\d+}}):
# CHECKERR: functions --outer --query f1
# CHECKERR: ^
# CHECKERR: (Type 'help functions' for related documentation)
echo $status
# CHECK: 2
functions --outer=bomboclat f1
# CHECKERR: functions: Invalid value for '--outer' option: 'bomboclat'. Expected 'current' or 'initial'
echo $status
# CHECK: 2
