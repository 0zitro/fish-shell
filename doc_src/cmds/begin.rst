begin - start a new block of code
=================================

Synopsis
--------

.. synopsis::

    begin; [COMMANDS ...]; end
    begin -q; [COMMANDS ...]; end
    begin -qq; [COMMANDS ...]; end
    begin -b; [COMMANDS ...]; end
    { [COMMANDS ...] }

Description
-----------

``begin`` is used to create a new block of code.

A block allows the introduction of a new :ref:`variable scope <variables-scope>`, redirection of the input or output of a set of commands as a group, or to specify precedence when using the conditional commands like ``and``.

The block is unconditionally executed. ``begin; ...; end`` is equivalent to ``if true; ...; end``.

``begin`` does not change the current exit status itself. After the block has completed, ``$status`` will be set to the status returned by the most recent command.

Some other shells only support the ``{ [COMMANDS ...] ; }`` notation.

The **-h** or **--help** option displays help about using this command.

Quoted blocks
-------------

A quoted option makes ``begin`` emit its body as source text instead of
executing it.  ``begin -q`` and ``begin --quoted`` evaluate command
substitutions eagerly and leave everything else suspended.  ``begin -qq``,
``begin -q -q``, and ``begin --quoted --quoted`` are equivalent spellings that
leave command substitutions suspended as well.  The modes differ only in
whether command substitutions are evaluated, not in what the value is.  A
quoted option is recognised by the token's value, as keywords and every other
option are: quoting, escaping, or a line continuation inside it does not
prevent it from selecting its mode.  A statement breaker before it keeps it a
body command.

::

    begin -q
        echo (date)
        echo $x
    end

This emits the body span: the bytes between the header and the closing ``end``,
with each command substitution replaced by its output.  Variables, braces,
globs, tildes, comments, indentation, and the structure of the body remain
source.  Nothing is trimmed and no terminator is added; the span carries the
author's own separators, so re-embedding it between ``begin`` and ``end``
reproduces the block byte for byte.

With two quoted options nothing is expanded:

::

    begin -qq
        echo (date)
    end

The span is a newline, four spaces, ``echo (date)``, and the newline before
``end``.  ``date`` is not run.  The body is still parsed so that nested blocks
and the matching ``end`` are found.

The emitted source can be captured or redirected like any other command output:

::

    set -l code "$(begin -q
        echo $x
    end)"
    eval $code

The emitted value is one element, whatever the statement count of the body.
Unquoted command substitution is the lossless capture: it yields the span as a
single element, including its breakers and trailing newlines::

    set -l code (begin -q
        echo $x
        echo $y
    end)
    eval $code

Quoted substitution yields the same text as one element, but it applies the
usual quoted-substitution trimming, so it is a convenience for text producers
rather than the lossless channel.  Consumers that want lines or statements take
that view explicitly, for example with ``string split``.

A redirection or pipeline applied to the construct itself receives the span
bytes: ``begin -q ... end > file`` writes the source text.  Redirections inside
the body remain part of the spanned source.  More than two quoted options is an
error.

Barrier blocks
--------------

The **-b** or **--barrier** option marks a block as code for the program that a
capture produces.  An enclosing capture emits the block's bytes - from its
``begin`` through the matching ``end`` - verbatim, and does not evaluate the
command substitutions inside it:

::

    begin -q
        begin -b
            echo (date)
        end
    end

The captured source contains the ``begin -b`` block unchanged, so ``date`` runs
when the emitted program executes the block, not during the capture.  A quoted
mode decides what a block does to its own body; the barrier decides what an
enclosing pass may do to the block.

A barrier combines with a quoted mode.  ``begin -b -q`` is a stage that expands
its body when the emitted program executes it, and ``begin -b -qq`` is a
quotation that emits its body as raw data at that point.  The combined spelling
is a shorthand for nesting the two blocks.

At runtime a bare barrier block only groups its body: it introduces no variable
scope, so ``set -l`` binds in the enclosing scope, and an explicit nested
``begin`` asks for block-local scope.  ``break``, ``continue`` and ``return``
pass through the grouping as they would through the body alone.

The flag covers the block's own bytes: a redirection written after its ``end``
is expanded by the enclosing capture.  To defer that redirection to the program,
wrap the whole statement in another barrier block.  More than one barrier option
is an error.

A command substitution whose program contains a barrier block is not evaluated
by the enclosing capture: the word containing it is emitted as written, and the
site expands when the emitted program runs.  The test is syntactic and
conservative - a barrier anywhere in the substitution's parsed program retains
the whole word.

Example
-------

The following code sets a number of variables inside of a block scope. Since the variables are set inside the block and have local scope, they will be automatically deleted when the block ends.

::

    begin
        set -l PIRATE Yarrr

        ...
    end

    echo $PIRATE
    # This will not output anything, since the PIRATE variable
    # went out of scope at the end of the block


In the following code, all output is redirected to the file out.html.

::

    begin
        echo $xml_header
        echo $html_header
        if test -e $file
            ...
        end
        ...
    end > out.html
