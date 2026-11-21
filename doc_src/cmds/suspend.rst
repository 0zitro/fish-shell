suspend - suspend the current shell or a stream
===============================================

Synopsis
--------

.. synopsis::

    suspend [--force]
    ... | suspend

Description
-----------

``suspend`` suspends execution of the current shell by sending it a SIGTSTP signal, returning to the controlling process. It can be resumed later by sending it a SIGCONT.  In order to prevent suspending a shell that doesn't have a controlling process, it will not suspend the shell if it is a login shell. This requirement is bypassed if the **--force** option is given or the shell is not interactive.

When the standard input is directly redirected, ``suspend`` records the shell's end of that input instead and writes a suspension handle of the form ``^N`` to its standard output.
The command itself does not block: the pipeline that feeds the input keeps running in the background, and a later :doc:`resume <resume>` waits for the stream and passes it on.
A stage after ``suspend`` receives the handle like any other output of the command.

A redirection on the ``suspend`` stage selects where the handle is written.

Example
-------

::

    > echo 1 | cat --show-all | suspend
    ^1
    > resume ^1
    1$
