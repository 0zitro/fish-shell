# RUN: fish=%fish %fish %s

# A quoted block emits its body span: the bytes between the header and the
# closing `end`, verbatim, as one element.  The breakers, comments, blank
# lines, and interior whitespace are the value, so each check captures the
# span and escapes it to make every byte visible.

echo (string escape -- (begin -qq echo 1; end))
# CHECK: ' echo 1; '

# -q and -qq emit the same framing; they differ only in substitution.
echo (string escape -- (begin -q echo 1; end))
# CHECK: ' echo 1; '

# A one-line body and the same body closed on the next line have different
# spans: the closing break is part of the value.
echo (string escape -- (begin -qq; code; end) (begin -qq; code
end))
# CHECK: '; code; ' \;\ code\n

# A body that starts on the header line and one that starts on the next line
# have different lead breakers.
echo (string escape -- (begin -qq echo 1; end) (begin -qq
echo 1; end))
# CHECK: ' echo 1; ' \necho\ 1\;\ 

# An empty body still has its span: the breakers alone.
echo (string escape -- (begin -qq end))
# CHECK: ' '
echo (string escape -- (begin -qq; end))
# CHECK: '; '
echo (string escape -- (begin -qq
end))
# CHECK: \n

# A comment is part of the span, including the newline that closes it.
echo (string escape -- (begin -qq # c
end))
# CHECK: \ \#\ c\n
echo (string escape -- (begin -qq
# c
end))
# CHECK: \n\#\ c\n

# A comment on the last line is followed by the newline that closes it, so no
# capture ends inside a comment.
echo (string escape -- (begin -qq
echo 1
# trailing
end))
# CHECK: \necho\ 1\n\#\ trailing\n

# Interior whitespace, blank lines, and indentation are the author's.
echo (string escape -- (begin -qq
echo 1

    echo 2
end))
# CHECK: \necho\ 1\n\n\ \ \ \ echo\ 2\n

# A continuation is emitted as its two bytes, never escaped.
echo (string escape -- (begin -qq \
code; end))
# CHECK: \ \\\ncode\;\ 

# The value is one element, whatever the statement count of the body.
set -l value (begin -qq
echo 1
echo 2
end)
echo (count $value)
# CHECK: 1
echo (string escape -- $value)
# CHECK: \necho\ 1\necho\ 2\n

# An empty body is one element holding its breaker span.
set -l empty (begin -qq end)
echo (count $empty) (string escape -- $empty)
# CHECK: 1 ' '

# Unquoted capture keeps the span's trailing newlines; quoted capture applies
# fish's quoted-substitution trimming, so it is not the lossless channel.
set -l quoted "$(begin -qq
echo 1
end)"
echo (count $quoted) (string escape -- $quoted)
# CHECK: 1 \necho\ 1

# Re-embedding the span between the header and `end` reproduces the block.
set -l span (begin -qq
echo 1

    echo 2
end)
echo (string escape -- $span)
# CHECK: \necho\ 1\n\n\ \ \ \ echo\ 2\n
echo (string escape -- (eval "begin -qq""$span""end"))
# CHECK: \necho\ 1\n\n\ \ \ \ echo\ 2\n

# A block's own redirection or pipeline receives the span bytes.
echo (string escape -- (begin -qq echo 1; end | cat))
# CHECK: ' echo 1; '

# Inner redirections remain part of the span.
echo (string escape -- (begin -qq echo (cmd) > inner; end))
# CHECK: ' echo (cmd) > inner; '

# Comments and nested blocks are span bytes.
echo (string escape -- (begin -qq
# comment
begin
    echo 1
end
end))
# CHECK: \n\#\ comment\nbegin\n\ \ \ \ echo\ 1\nend\n

# In -q mode only command-substitution sites change; every other byte is the
# span verbatim.
echo (string escape -- (begin -q echo (printf x); end))
# CHECK: ' echo x; '

echo (string escape -- (begin -q echo pre(printf 'a\nb')post; end))
# CHECK: ' echo preapost prebpost; '

echo (string escape -- (begin -q echo "$(printf 'a\nb')"; end))
# CHECK: \ echo\ \"a\nb\"\;\ 

echo (string escape -- (begin -q echo (printf ''); end))
# CHECK: ' echo ; '

echo (string escape -- (begin -q echo (printf 'a\nb') (printf 'c\nd'); end))
# CHECK: ' echo a b c d; '

# Comments, variables, braces, globs, and tildes remain span bytes in -q mode.
echo (string escape -- (begin -q
# comment
set x 1
echo $x
end))
# CHECK: \n\#\ comment\nset\ x\ 1\necho\ \$x\n

echo (string escape -- (begin -q echo {a,b} ~; end))
# CHECK: ' echo {a,b} ~; '

echo (string escape -- (begin -q echo {pre(printf 'a,b')}; end))
# CHECK: " echo {pre'a,b'}; "

echo (string escape -- (begin -q echo 1 > (printf out); end))
# CHECK: ' echo 1 > out; '

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

# An eager substitution inside a continuation-joined word is rendered; the
# continuation is inside the substituted word, not span trivia.
set foo 1
set code (begin -q echo $foo\
(printf bar); end)
eval $code
# CHECK: 1bar

# A raw block keeps its body unexpanded.
echo (string escape -- (begin -qq echo (printf x); end))
# CHECK: ' echo (printf x); '

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

# A dynamic variable reference must be exactly one reference.
$fish -c 'begin -q $$(printf "x\ny\n"); end'
# CHECKERR: fish: Dynamic variable reference must be exactly one variable reference
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

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
