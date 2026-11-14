fish_indent - indenter and prettifier
=====================================

Synopsis
--------

.. synopsis::

    fish_indent [OPTIONS] [FILE ...]


Description
-----------

:program:`fish_indent` is used to indent a piece of fish code. :program:`fish_indent` reads commands from standard input or the given filenames and outputs them to standard output or a specified file (if ``-w`` is given).

The following options are available:

**-w** or **--write**
    Indents a specified file and immediately writes to that file.

**-i** or **--no-indent**
    Do not indent commands; only reformat to one job per line.

**--only-indent**
    Do not reformat, only indent each line.

**--only-unindent**
    Do not reformat, only unindent each line.

**-c** or **--check**
    Do not indent, only return 0 if the code is already indented as fish_indent would, the number of failed files otherwise. Also print the failed filenames if not reading from standard input.

**-v** or **--version**
    Displays the current :program:`fish` version and then exits.

**--ansi**
    Colorizes the output using ANSI escape sequences using the colors defined in the environment (such as :envvar:`fish_color_command`).

**--html**
    Outputs HTML, which supports syntax highlighting if the appropriate CSS is defined. The CSS class of a piece of text is the name of the color variable that colors it: ``fish_color_param``, ``fish_color_end``. Every class is a variable name, so a stylesheet is written against the same names the configuration uses.

**--pygments**
    Outputs one line per run of characters in the input, as ``start,end,path,enclosing,attributes``. ``path`` is the dotted path of the scope that colors the run (``command.external``), ``enclosing`` is the paths of the scopes that enclose it, most general first, joined by ``+``, and ``attributes`` is the variables that the run's attributes carry, joined by ``+``. The ranges are half-open and count characters.

**--io**
    Lets the highlighting that ``--ansi``, ``--html`` and ``--pygments`` report resolve commands and paths, the way the command line does: a builtin, a function and a program each take their own color, and a path that exists takes its attribute. Without it the pass resolves nothing, so a command word takes the color every kind falls back on and no path is tested.

**--dump-parse-tree**
    Dumps information about the parsed statements to standard error. This is likely to be of interest only to people working on the fish source code.

**-h** or **--help**
    Displays help about using this command.
