suspend - suspend the current shell or a stream
===============================================

Synopsis
--------

.. synopsis::

    suspend [--force]
    suspend [--fd FD] ... SOURCE ...
    ... | suspend

Description
-----------

``suspend`` suspends execution of the current shell by sending it a SIGTSTP signal, returning to the controlling process. It can be resumed later by sending it a SIGCONT.  In order to prevent suspending a shell that doesn't have a controlling process, it will not suspend the shell if it is a login shell. This requirement is bypassed if the **--force** option is given or the shell is not interactive.

When the standard input is directly redirected, ``suspend`` records the shell's end of that input instead and writes a suspension handle of the form ``^N`` to its standard output.
The command itself does not block: the pipeline that feeds the input keeps running in the background, and a later :doc:`resume <resume>` waits for the stream and passes it on.
A stage after ``suspend`` receives the handle like any other output of the command.

With operands, the **command form** starts the operands as one pipeline.
The operands are joined with spaces into one source string, as the operands of ``eval`` are, and must contain one pipeline.
The pipeline runs as one job with its standard descriptors suspended:

- descriptor 0 of the first stage reads a pipe whose write end the shell records;
- descriptor 1 of the last stage writes a pipe whose read end the shell records;
- descriptor 2 of every stage writes one pipe whose read end the shell records.

``--fd FD`` suspends one more descriptor of the pipeline: by default the pipeline writes ``FD`` and the shell records the read end; ``--fd FD:in`` makes the pipeline read ``FD`` and the shell records the write end.
Each bound descriptor is one pipe shared by every stage that uses it.
The handle is written to the standard output of ``suspend``; a redirection on the ``suspend`` command applies to the handle, not to the started pipeline.

A redirection on the ``suspend`` stage selects where the handle is written.

Example
-------

::

    > echo 1 | cat --show-all | suspend
    ^1
    > resume ^1
    1$

    > set -l h (suspend --fd 5 'echo one >&5 | echo two >&5')
    > resume $h 5>| command sort
    one
    two
