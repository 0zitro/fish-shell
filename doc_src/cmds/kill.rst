kill - send a signal to a process or job
=========================================

Synopsis
--------

.. synopsis::

    kill [-s SIGNAL | -SIGNAL] PID ...
    kill [-s SIGNAL | -SIGNAL] JOB ...
    kill -l [SIGNAL | 0xMASK]
    kill -L

Description
-----------

``kill`` sends ``SIGNAL`` to each target process or job.
``SIGNAL`` is a signal name with or without the ``SIG`` prefix, or a signal number; the default is ``TERM``.
Signal 0 performs the permission and existence checks without sending a signal.

A target is a process identifier or a fish job specification such as ``%1``.
A positive identifier names one process, ``0`` names the caller's process group, and a negative identifier names the process group with that absolute identifier.
The ``kill`` function expands a fish job specification to the identifiers of the job's external processes before the builtin runs.

``-l`` prints the signal names, converts one signal number or name to a name, or decodes a hexadecimal signal mask.
``-L`` prints the signal names with their numbers.
The status is 0 when every target succeeded, 1 when every target failed, and 64 when some succeeded and some failed.

Options
-------

**-s** SIGNAL, **--signal** SIGNAL
    Send ``SIGNAL`` instead of ``TERM``.

**-SIGNAL**
    Shorthand for ``-s SIGNAL``, for example ``-TERM`` or ``-9``.

**-l** [SIGNAL], **--list** [SIGNAL]
    List the signal names, or convert ``SIGNAL`` to a name.
    An argument beginning with ``0x`` is a hexadecimal signal mask; each set bit names one signal.

**-L**, **--table**
    List every signal name with its number.

**-h**, **--help**
    Display help and exit.

**-V**, **--version**
    Display the version and exit.

Examples
--------

::

    > sleep 30 &
    > kill %1
    > wait %1
    echo $status

The job receives the default ``TERM`` signal, and ``wait`` reports the status of its termination.

::

    > kill -l 15
    TERM
    > kill -l 0x4000
    TERM

The first command converts signal number 15 to its name; the second decodes the ninth signal bit in a mask.
