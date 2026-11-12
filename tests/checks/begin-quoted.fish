# RUN: fish=%fish %fish %s

# Raw quoted begin emits its body without expansion.
begin -qq echo (printf x); end
# CHECK: echo (printf x)

begin -q -q echo $x; end
# CHECK: echo $x

begin --quoted --quoted echo 1; echo 2; end
# CHECK: echo 1; echo 2

begin -qq; echo 1; end
# CHECK: echo 1

# The block's own redirections and pipelines apply to the emitted source.
begin -qq echo 1; end | cat
# CHECK: echo 1

# Inner redirections remain part of the source.
begin -qq echo (cmd) > inner; end
# CHECK: echo (cmd) > inner

# Comments and nested blocks are preserved.
begin -qq
# comment
begin
    echo 1
end
end
# CHECK: # comment
# CHECK: begin
# CHECK:     echo 1
# CHECK: end

# More than two quoted options is an error.
$fish -c 'begin -qq -q echo 1; end'
# CHECKERR: fish: begin accepts at most two quoted options
# CHECKERR: {{^}}begin -qq -q echo 1; end
# CHECKERR: {{^}}          ^^

# A compact spelling carries one option per q, so a third q is an error too.
$fish -c 'begin -qqq echo 1; end'
# CHECKERR: fish: begin accepts at most two quoted options
# CHECKERR: {{^}}begin -qqq echo 1; end
# CHECKERR: {{^}}      ^~~^

# A single quoted option evaluates command substitutions only.
begin -q echo (printf x); end
# CHECK: echo x

begin -q echo (printf 'a\nb\n')[2]; end
# CHECK: echo b

begin -qq echo foo\; ; end
# CHECK: echo foo\;

begin -q $$(printf 'x[1]\n'); end
# CHECK: $x[1]

$fish -c 'begin -q $$(printf "x\ny\n"); end'
# CHECKERR: fish: Dynamic variable reference must be exactly one variable reference
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

begin -q echo pre(printf 'a\nb')post; end
# CHECK: echo preapost prebpost

begin -q echo (printf 'a\nb') (printf 'c\nd'); end
# CHECK: echo a b c d

begin -q echo "$(printf '')"; end
# CHECK: echo ""

begin -q echo (printf ''); end
# CHECK: echo

begin -q echo "$(printf 'a\nb')"; end
# CHECK: echo "a
# CHECK: b"

begin -q
# comment
set x 1
echo $x
end
# CHECK: # comment
# CHECK: set x 1
# CHECK: echo $x

begin -q echo {a,b} ~; end
# CHECK: echo {a,b} ~

begin -q echo {pre(printf 'a,b')}; end
# CHECK: echo {pre'a,b'}

begin -q echo 1 > (printf out); end
# CHECK: echo 1 > out

# A command-substitution result keeps the boundary of the value beside it.
set foo 1
eval (begin -q echo $foo(printf bar); end)
# CHECK: 1bar
echo $foo(printf bar)
# CHECK: 1bar
eval (begin -q echo "$foo$(printf bar)"; end)
# CHECK: 1bar
set foo bar
set bar 2
eval (begin -q echo $$foo(printf x); end)
# CHECK: 2x

# A `$^N(...)` operand is a complete reference; following source is a suffix.
set n x
set x 123
eval (begin -q echo $$(echo n)post; end)
# CHECK: xpost

# Line continuations are transparent to the captured value boundary.
set foo 1
set code (begin -q echo $foo\
(printf bar); end | string collect)
eval $code
# CHECK: 1bar

# A later substitution reports its own span, not the token start.
$fish -c "begin -q echo (printf a)(printf b)[0]; end"
# CHECKERR: fish: array indices start at 1, not 0.
# CHECKERR: {{^}}begin -q echo (printf a)(printf b)[0]; end
# CHECKERR: {{^}}                                   ^

# An eager-serialisation error points at the substitution that failed.
$fish -c "begin -q echo (printf 'a')[0]; end"
# CHECKERR: fish: array indices start at 1, not 0.
# CHECKERR: {{^}}begin -q echo (printf 'a')[0]; end
# CHECKERR: {{^}}                           ^
