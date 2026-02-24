functions - print or erase functions
====================================

Synopsis
--------

.. synopsis::

    functions [-a | --all] [-n | --names] [--color WHEN]
    functions [-D | --details] [-v] [--color WHEN] FUNCTION
    functions [--no-details[=MODE]] [--color WHEN] FUNCTION ...
    functions [-o | --outer[=MODE]] FUNCTION
    functions -c OLDNAME NEWNAME
    functions -d DESCRIPTION FUNCTION
    functions [-e | -q] FUNCTION ...

Description
-----------

``functions`` prints or erases functions.

The following options are available:

**-a** or **--all**
    Lists all functions, even those whose name starts with an underscore.

**-c** or **--copy** *OLDNAME* *NEWNAME*
    Creates a new function named *NEWNAME*, using the definition of the *OLDNAME* function.

**-d** or **--description** *DESCRIPTION*
    Changes the description of this function.

**-e** or **--erase**
    Causes the specified functions to be erased. This also means that it is prevented from autoloading in the current session. Use :doc:`funcsave <funcsave>` to remove the saved copy.

**-D** or **--details**
    Reports the path name where the specified function is defined or could be autoloaded, ``stdin`` if the function was defined interactively or on the command line or by reading standard input, **-** if the function was created via :doc:`source <source>`, and ``n/a`` if the function isn't available. (Functions created via :doc:`alias <alias>` will return **-**, because ``alias`` uses ``source`` internally. Copied functions will return where the function was copied.) If the **--verbose** option is also specified then five lines are written:

    - the path name as already described,
    - if the function was copied, the path name to where the function was originally defined, otherwise ``autoloaded``, ``not-autoloaded`` or ``n/a``,
    - the line number within the file or zero if not applicable,
    - ``scope-shadowing`` if the function shadows the vars in the calling function (the normal case if it wasn't defined with **--no-scope-shadowing**), ``no-scope-shadowing`` for function-level scope sharing, ``transparent`` for fully transparent local scope sharing, or ``n/a`` if the function isn't defined,
    - the function description minimally escaped so it is a single line, or ``n/a`` if the function isn't defined or has no description.

    You should not assume that only five lines will be written since we may add additional information to the output in the future.

**-o** or **--outer**\ [**=**\ *MODE*]
    Reports outer-function provenance for *FUNCTION*. This reflects runtime provenance for the latest loaded definition and does not autoload the target function or its outer function.

    *MODE* may be:

    - ``current`` (default): report the currently captured outer function for the loaded function generation.
    - ``initial``: report the first recorded definition-site outer function name for the currently loaded function name.

    On success, one line is written containing the outer function name.

    If the target exists but has no outer function (for example, it was defined at top level), this command returns status 1.

    If the target exists but the requested outer function is no longer available, this command returns status 3.

    If provenance is ambiguous, this command returns status 4.

    If the target function is not currently loaded, this command returns status 5.

**--no-details**\ [**=**\ *MODE*]
    Turns off function path reporting. By default this prints the full function definition.

    *MODE* may be:

    - ``definition-only`` (default): print only the function definition, without path metadata.
    - ``body-only``: print only the function body exactly as sourced, omitting the ``function ...`` header and ``end``.

**-n** or **--names**
    Lists the names of all defined functions.

**-q** or **--query**
    Tests if the specified functions exist.

**-v** or **--verbose**
    Make some output more verbose.

**-H** or **--handlers**
    Show all event handlers.

**-t** or **--handlers-type** *TYPE*
    Show all event handlers matching the given *TYPE*.

**--color** *WHEN*
    Controls when to use syntax highlighting colors when printing function definitions.
    *WHEN* can be ``auto`` (the default, colorize if the output :doc:`is a terminal <isatty>`), ``always``, or ``never``.

**-h** or **--help**
    Displays help about using this command.

The default behavior of ``functions``, when called with no arguments, is to print the names of all defined functions. Unless the ``-a`` option is given, no functions starting with underscores are included in the output.

If any non-option parameters are given, the definition of the specified functions are printed.

Copying a function using ``-c`` copies only the body of the function, and does not attach any event notifications from the original function.

Only one function's description can be changed in a single invocation of ``functions -d``.

The exit status of ``functions`` is the number of functions specified in the argument list that do not exist, which can be used in concert with the ``-q`` option.


Examples
--------


::

    functions -n
    # Displays a list of currently-defined functions

    functions -c foo bar
    # Copies the 'foo' function to a new function called 'bar'

    functions -e bar
    # Erases the function ``bar``

See more
--------

For more explanation of how functions fit into fish, see :ref:`Functions <syntax-function>`.
