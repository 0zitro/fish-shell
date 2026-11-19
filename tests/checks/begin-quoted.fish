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

# In -q mode each eager substitution site is reified, so evaluating the
# capture reproduces the direct command's arguments.
eval (begin -q echo (printf x); end)
# CHECK: x

eval (begin -q echo pre(printf 'a\nb')post; end)
# CHECK: preapost prebpost

eval (begin -q echo "$(printf 'a\nb')"; end) | string escape
# CHECK: a
# CHECK: b

set -l empty_word (eval (begin -q echo (printf ''); end))
count $empty_word
# CHECK: 1

eval (begin -q echo (printf 'a\nb') (printf 'c\nd'); end)
# CHECK: a b c d

# Comments, variables, braces, globs, and tildes remain span bytes in -q mode.
echo (string escape -- (begin -q
# comment
set x 1
echo $x
end))
# CHECK: \n\#\ comment\nset\ x\ 1\necho\ \$x\n

echo (string escape -- (begin -q echo {a,b} ~; end))
# CHECK: ' echo {a,b} ~; '

eval (begin -q echo {pre(printf 'a,b')}; end)
# CHECK: {prea,b}

set -l target (mktemp)
eval (begin -q echo 1 > (printf '%s' $target); end)
cat $target
# CHECK: 1
rm -f $target

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

# A quoted option is recognised by the token's value, as keywords and every
# other option are: quoting, escaping, and line continuations do not change
# what it denotes, so they do not prevent it from selecting its mode.
echo (string escape -- (begin '-q'
echo 1; end))
# CHECK: \necho\ 1\;\ 

echo (string escape -- (begin "-q" echo 1; end))
# CHECK: ' echo 1; '

echo (string escape -- (begin \-q
echo 1; end))
# CHECK: \necho\ 1\;\ 

echo (string escape -- (begin '--quoted' echo 1; end))
# CHECK: ' echo 1; '

echo (string escape -- (begin \
-q
echo 1; end))
# CHECK: \necho\ 1\;\ 

echo (string escape -- (begin -\
q
echo 1; end))
# CHECK: \necho\ 1\;\ 

echo (string escape -- (begin -q\
q
echo 1; end))
# CHECK: \necho\ 1\;\ 

echo (string escape -- (begin --quo\
ted
echo 1; end))
# CHECK: \necho\ 1\;\ 

# A statement breaker before the token keeps it a body command.
$fish -c 'begin; -q; end'
# CHECKERR: fish: Unknown command: -q
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

$fish -c "begin; '-q'; end"
# CHECKERR: fish: Unknown command: -q
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

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

# A barrier option is a header option: it is consumed before the body, like a
# quoted option, and never runs as a command.  A barrier combines with a quoted
# mode; the quoted mode still selects the emitted span.
echo (string escape -- (begin -bq echo 1; end))
# CHECK: ' echo 1; '

echo (string escape -- (begin -qb echo 1; end))
# CHECK: ' echo 1; '

echo (string escape -- (begin -bqq echo 1; end))
# CHECK: ' echo 1; '

echo (string escape -- (begin -qqb echo 1; end))
# CHECK: ' echo 1; '

echo (string escape -- (begin -b -q echo 1; end))
# CHECK: ' echo 1; '

echo (string escape -- (begin --barrier --quoted --quoted echo 1; end))
# CHECK: ' echo 1; '

# A bare barrier header is consumed too; the body is the block's body.
$fish -c 'begin -b; echo 1; end'
# CHECK: 1

$fish -c 'begin --barrier; echo 1; end'
# CHECK: 1

# A barrier with two quoted options still emits the raw span.
echo (string escape -- (begin -bqq echo (printf x); end))
# CHECK: ' echo (printf x); '

# Barrier recognition is by token value, like every other header option.
echo (string escape -- (begin '-bq' echo 1; end))
# CHECK: ' echo 1; '

echo (string escape -- (begin "-bq" echo 1; end))
# CHECK: ' echo 1; '

echo (string escape -- (begin \-bq echo 1; end))
# CHECK: ' echo 1; '

echo (string escape -- (begin --bar\
rier -q echo 1; end))
# CHECK: ' echo 1; '

# A barrier option is a header option for the span rule: the span begins after
# it, so a continuation inside the token is not body bytes.
echo (string escape -- (begin -b\
q echo 1; end))
# CHECK: ' echo 1; '

# A statement breaker before the token keeps it a body command.
$fish -c 'begin; -b; end'
# CHECKERR: fish: Unknown command: -b
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

$fish -c 'begin; -bq; end'
# CHECKERR: fish: Unknown command: -bq
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}
# CHECKERR: {{.*}}

# More than one barrier option is an error.
$fish -c 'begin -bb echo 1; end'
# CHECKERR: fish: begin accepts at most one barrier option
# CHECKERR: {{^}}begin -bb echo 1; end
# CHECKERR: {{^}}      ^~^

$fish -c 'begin -b -b echo 1; end'
# CHECKERR: fish: begin accepts at most one barrier option
# CHECKERR: {{^}}begin -b -b echo 1; end
# CHECKERR: {{^}}         ^^

$fish -c 'begin --barrier --barrier echo 1; end'
# CHECKERR: fish: begin accepts at most one barrier option
# CHECKERR: {{^}}begin --barrier --barrier echo 1; end
# CHECKERR: {{^}}                ^~~~~~~~^

# A third quoted option is an error with a barrier present, however it is
# spelled.
$fish -c 'begin -bqqq echo 1; end'
# CHECKERR: fish: begin accepts at most two quoted options
# CHECKERR: {{^}}begin -bqqq echo 1; end
# CHECKERR: {{^}}      ^~~~^

$fish -c 'begin -b -q -q -q echo 1; end'
# CHECKERR: fish: begin accepts at most two quoted options
# CHECKERR: {{^}}begin -b -q -q -q echo 1; end
# CHECKERR: {{^}}               ^^

# A barrier block is source for the emitted program: an enclosing -q capture
# copies its bytes verbatim and expands no substitution site inside it.
echo (string escape -- (begin -q
    begin -b
        echo (printf inner)
    end
end))
# CHECK: \n\ \ \ \ begin\ -b\n\ \ \ \ \ \ \ \ echo\ \(printf\ inner\)\n\ \ \ \ end\n

# Eager expansion outside the barrier is reified; the barrier body runs when
# the capture is evaluated.
eval (begin -q
    echo (printf outer)
    begin -b
        echo (printf inner)
    end
end)
# CHECK: outer
# CHECK: inner

# The block is opaque at every depth in the body's block structure, including
# inside a nested quoted block.
echo (string escape -- (begin -q
    begin -qq
        begin -b
            echo (printf inner)
        end
    end
end))
# CHECK: \n\ \ \ \ begin\ -qq\n\ \ \ \ \ \ \ \ begin\ -b\n\ \ \ \ \ \ \ \ \ \ \ \ echo\ \(printf\ inner\)\n\ \ \ \ \ \ \ \ end\n\ \ \ \ end\n

# A barrier combined with a quoted mode is opaque as a whole: its quoted mode
# runs when the emitted program reaches the block.
echo (string escape -- (begin -q
    begin -b -q
        echo (printf inner)
    end
end))
# CHECK: \n\ \ \ \ begin\ -b\ -q\n\ \ \ \ \ \ \ \ echo\ \(printf\ inner\)\n\ \ \ \ end\n

echo (string escape -- (begin -q
    begin -b -qq
        echo (printf inner)
    end
end))
# CHECK: \n\ \ \ \ begin\ -b\ -qq\n\ \ \ \ \ \ \ \ echo\ \(printf\ inner\)\n\ \ \ \ end\n

# The flag stays in the emitted source, so a second capture pass applies the
# same rule.
set -l barrier_source (begin -q
    begin -b
        echo (printf inner)
    end
end)
set -l rescanned (eval "begin -q
$barrier_source
end")
echo (string escape -- $rescanned)
# CHECK: \n\n\ \ \ \ begin\ -b\n\ \ \ \ \ \ \ \ echo\ \(printf\ inner\)\n\ \ \ \ end\n\n

# The flag covers the block's own bytes: a redirection after its `end` is
# reified by the enclosing pass, and the reified target is used when the
# capture is evaluated.
set -l target (mktemp)
eval (begin -q
    set -l marker program
    begin -b
        echo (printf inner)
    end > (printf %s $target)
end)
cat $target
# CHECK: inner
rm -f $target

echo (string escape -- (begin -q
    set -l marker program
    begin -b
        begin -b
            echo (printf inner)
        end > (printf %s $marker)
    end
end))
# CHECK: \n\ \ \ \ set\ -l\ marker\ program\n\ \ \ \ begin\ -b\n\ \ \ \ \ \ \ \ begin\ -b\n\ \ \ \ \ \ \ \ \ \ \ \ echo\ \(printf\ inner\)\n\ \ \ \ \ \ \ \ end\ \>\ \(printf\ \%s\ \$marker\)\n\ \ \ \ end\n

# Evaluating a captured barrier block runs its mode.
set -l executed (begin -q
    begin -b
        echo bare
    end
end)
eval $executed
# CHECK: bare

set -l staged (begin -q
    set -l marker program
    begin -b -q
        echo $marker (printf inner)
    end
end)
set -l staged_source (eval $staged)
set marker program
eval $staged_source
# CHECK: program inner

set -l raw_staged (begin -q
    set -l marker program
    begin -b -qq
        echo $marker (printf inner)
    end
end)
eval $raw_staged
# CHECK: echo $marker (printf inner)

# A bare barrier block is transparent at runtime: its body binds in the
# enclosing scope, and an explicit nested begin supplies block-local scope.
function barrier_scope
    begin -b
        set -l barrier_local inner
    end
    echo $barrier_local
end
barrier_scope
# CHECK: inner

function barrier_nested_scope
    begin -b
        begin
            set -l nested_local inner
        end
    end
    set -q nested_local; echo $status
end
barrier_nested_scope
# CHECK: 1

function plain_scope
    begin
        set -l plain_local inner
    end
    set -q plain_local; echo $status
end
plain_scope
# CHECK: 1

# The transparent grouping leaves loop control and return alone.
for i in 1 2 3
    begin -b
        if test $i -eq 2
            continue
        end
        echo $i
    end
end
# CHECK: 1
# CHECK: 3

for i in 1 2 3
    begin -b
        if test $i -eq 2
            break
        end
        echo $i
    end
end
# CHECK: 1

function barrier_return
    begin -b
        return 7
    end
    echo unreachable
end
barrier_return
echo $status
# CHECK: 7

# The combined spelling is a shorthand for the nesting: the two programs run
# the same inner block.
set -l combined (begin -q
    begin -b -q
        echo (printf inner)
    end
end)
set -l nested (begin -q
    begin -b
        begin -q
            echo (printf inner)
        end
    end
end)
set -l combined_output (eval $combined)
set -l nested_output (eval $nested)
eval $combined_output
eval $nested_output
# CHECK: inner
# CHECK: inner

# A command substitution whose program contains a barrier block is forwarded as
# written, and its eager peers in the word are reified.  Evaluating the capture
# runs every site at program time.
set -l forwarded_site (begin -q echo (begin -b; echo x; end); end)
eval $forwarded_site
# CHECK: x

set -l forwarded_pair (begin -q echo (begin -b; echo x; end)(printf suffix); end)
eval $forwarded_pair
# CHECK: xsuffix

# The test is conservative: a barrier inside a quoted part of the site's
# program forwards the site too.
echo (string escape -- (begin -q echo (begin -qq; begin -b; echo x; end; end); end))
# CHECK: ' echo (begin -qq; begin -b; echo x; end; end); '

# Executing a site executes the substitutions inside it, so a barrier in a
# nested substitution forwards the outer site as well.
echo (string escape -- (begin -q echo (echo (begin -b; echo x; end)); end))
# CHECK: ' echo (echo (begin -b; echo x; end)); '

# A forwarded site does not keep the eager sites around it as written.
set -l mixed_word (begin -q echo (printf a)(begin -b; echo x; end)(printf b); end)
eval $mixed_word
# CHECK: axb

# A forwarded site composes with eager peers and literal brace code in the
# direct execution's order.
eval (begin -q echo (begin -b; printf 'X\nY'; end)(printf 'c\nd'); end)
# CHECK: Xc Xd Yc Yd

eval (begin -q echo (begin -b; printf 'X\nY'; end){a,b}; end)
# CHECK: Xa Xb Ya Yb

# A barrier-free site stays eager and its capture evaluates to the same value.
eval (begin -q echo (printf eager); end)
# CHECK: eager

# The flag survives a second capture pass in a substitution's source too.
set -l site_source (begin -q
    echo (begin -b; echo x; end)
end)
set -l site_rescanned (eval "begin -q
$site_source
end")
echo (string escape -- $site_rescanned)
# CHECK: \n\n\ \ \ \ echo\ \(begin\ -b\;\ echo\ x\;\ end\)\n\n

# Evaluating the capture expands the retained sites at program time.
set -l deferred_word (begin -q
    echo pre(begin -b; echo x; end)post
end)
eval $deferred_word
# CHECK: prexpost

set -l deferred_pair (begin -q
    echo (printf left)(begin -b; printf mid; end)(printf right)
end)
eval $deferred_pair
# CHECK: leftmidright
