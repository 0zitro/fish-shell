resume - consume a suspended stream
===================================

Synopsis
--------

.. synopsis::

    resume HANDLE

Description
-----------

``resume`` consumes the suspension that ``HANDLE`` denotes.
A suspension is the shell's end of the pipe whose other end a suspended command holds: the :doc:`suspend <suspend>` command records it and writes the handle.
``resume`` links that pipe to its own standard output, so the suspended stream reaches wherever the command's output goes and a pipeline stage after ``resume`` reads it directly.

The stream is relayed in the kernel with ``splice(2)``; a destination the kernel cannot splice to, such as a terminal, receives the bytes from a user space copy.
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
