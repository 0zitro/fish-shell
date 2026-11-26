resume - consume a suspended stream
===================================

Synopsis
--------

.. synopsis::

    resume HANDLE

Description
-----------

``resume`` consumes the suspension that ``HANDLE`` denotes.
A suspension is the shell's end of a pipe whose other end a suspended command holds: the :doc:`suspend <suspend>` command records it and writes the handle.
``resume`` links every recorded descriptor to its own descriptor of the same number, so the suspended stream goes wherever the command's descriptor points.

For a descriptor the suspended pipeline writes, the shell holds the read end and ``resume`` writes the stream to its own descriptor of that number: to its standard output for descriptor 1, to its standard error for descriptor 2, and to the descriptor the command's own redirections give it for any other number.
For a descriptor the suspended pipeline reads, the shell holds the write end and ``resume`` feeds it from its own descriptor of that number, which the command opens with an ordinary input redirection.
A stream suspension records descriptor 0 only and writes that stream to the standard output.

A pipeline stage after ``resume`` reads the standard output as for any command, so ``resume $h | command`` consumes the suspended standard output through the command.
A descriptor selected with a pipe redirection is connected the same way: ``resume $h 5>| command`` writes the suspended descriptor 5 to ``command``'s standard input.
The recorded descriptors are relayed in the kernel with ``splice(2)``; a destination the kernel cannot splice to, such as a terminal, receives the bytes from a user space copy.
``resume`` blocks while the suspended command produces the stream, as the command would in an ordinary pipeline.

A handle denotes one suspension.
``resume`` consumes it, so a later ``resume`` of the same handle reports that the shell records no such suspension.
A failed ``resume`` leaves the handle usable.

Example
-------

::

    > echo 1 | cat --show-all | suspend
    ^1
    > resume ^1 | cat --show-all
    1$$

    > set -l h (suspend --fd 5 'echo one >&5 | echo two >&5')
    > resume $h 5>| command sort
    one
    two
