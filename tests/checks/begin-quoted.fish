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

# A single quoted option evaluates command substitutions only.
begin -q echo (printf x); end
# CHECK: echo x

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

begin -q echo 1 > (printf out); end
# CHECK: echo 1 > out
