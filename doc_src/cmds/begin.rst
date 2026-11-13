begin - start a new block of code
=================================

Synopsis
--------

.. synopsis::

    begin; [COMMANDS ...]; end
    begin -q; [COMMANDS ...]; end
    begin -qq; [COMMANDS ...]; end
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
leave command substitutions suspended as well.

::

    begin -q
        echo (date)
        echo $x
    end

This emits ``echo <date output>`` and the literal line ``echo $x``.  Variables,
braces, globs, tildes, and the structure of the body remain source; only
command substitutions are replaced by their output.

With two quoted options nothing is expanded:

::

    begin -qq
        echo (date)
    end

This writes ``echo (date)`` without running ``date``.  The body is still parsed
so that nested blocks and the matching ``end`` are found.

The emitted text can be captured or redirected like any other command output:

::

    set -l code "$(begin -q
        echo $x
    end)"
    eval $code

A redirection or pipeline applied to the construct itself applies to the
emitted text.  Redirections inside the body remain part of the emitted source.
More than two quoted options is an error.

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
