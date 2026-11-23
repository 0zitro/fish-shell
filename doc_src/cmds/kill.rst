kill - send a signal to a process or job
=========================================

Synopsis
--------

.. synopsis::

    kill [-s SIGNAL | -SIGNAL] [-a] [-p] [-r] [--verbose] PID | NAME ...
    kill [-s SIGNAL | -SIGNAL] JOB ...
    kill -l [SIGNAL | 0xMASK]
    kill -L
    kill -d PID

Description
-----------

``kill`` sends ``SIGNAL`` to each target process or job.
``SIGNAL`` is a signal name with or without the ``SIG`` prefix, or a signal number; the default is ``TERM``.
Signal 0 performs the permission and existence checks without sending a signal.

A target is a process identifier or a process name.
A positive identifier names one process, ``0`` names the caller's process group, and a negative identifier names the process group with that absolute identifier.
A process name matches the process name in ``/proc/PID/stat``; only processes owned by the caller are matched unless ``--all`` is given.
A fish job specification such as ``%1`` is expanded by the ``kill`` function to the identifiers of the job's external processes before the builtin runs.

``-l`` prints the signal names, converts one signal number or name to a name, or decodes a hexadecimal signal mask.
``-L`` prints the signal names with their numbers.
The status is 0 when every target succeeded, 1 when every target failed, and 64 when some succeeded and some failed.

Options
-------

**-s** SIGNAL, **--signal** SIGNAL
    Send ``SIGNAL`` instead of ``TERM``.

**-SIGNAL**
    Shorthand for ``-s SIGNAL``, for example ``-TERM`` or ``-9``.

**-a**, **--all**
    Match process names owned by every user, not only the caller.

**-p**, **--pid**
    Print the resolved process identifiers without sending a signal.
    This is mutually exclusive with a signal option.

**-r**, **--require-handler**
    Do not signal a process that has no userspace handler for the signal.
    This uses the ``SigCgt`` field of ``/proc/PID/stat`` and is available on Linux.

**-d** PID, **--show-process-state** PID
    Print the signal-related fields of ``/proc/PID/status``.
    This is available on Linux.

**--verbose**
    Print each signal and process identifier before acting.

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
    > kill -p sleep
    123
    > kill sleep

The first command prints the process identifier of each matching ``sleep`` process without signalling it; the second sends the default ``TERM`` signal to those processes.

::

    > kill -l 15
    TERM
    > kill -l 0x4000
    TERM

The first command converts signal number 15 to its name; the second decodes the ninth signal bit in a mask.
