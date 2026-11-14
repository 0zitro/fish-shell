# This is a plugin for pygments that shells out to fish_indent.

# Example of how to use this:
# env PATH="/dir/containing/fish/indent/:$PATH" pygmentize -f terminal256 -l /path/to/fish_indent_lexer.py:FishIndentLexer -x ~/test.fish

from pygments.lexer import Lexer
from pygments.token import (
    Name,
    Comment,
    String,
    Error,
    Number,
    Operator,
    Other,
    Generic,
    Whitespace,
    String,
    Text,
    Punctuation,
)
import re
import subprocess

# A fallback token type.
DEFAULT = Text

# Mapping from fish token-scope paths to Pygments types.
#
# Only a family needs to be mapped: a refinement inherits the token of the family it
# names, so a scope the declaration adds under one of them needs no entry here.
SCOPE_TO_TOKEN = {
    "source": Name.Variable,
    "comment": Comment,
    "keyword": Name.Function,
    "command": Name.Function,
    "argument": Name.Constant,
    "string.quoted": String,
    "string.quoted.single": String.Single,
    "string.quoted.double": String.Double,
    "string.escape": String.Escape,
    "string.escape.invalid": Generic.Error,
    "variable": Name.Variable,
    "operator": Operator,
    "redirection": Punctuation,
    "terminator": Punctuation,
    "invalid": Generic.Error,
}


def token_for_scope(scope):
    """Return the pygments token for a fish token-scope path.

    A scope that has no entry of its own is drawn as the family it is named under, and
    a scope outside every mapped family is drawn as plain text.
    """
    while scope:
        token = SCOPE_TO_TOKEN.get(scope)
        if token is not None:
            return token
        scope = scope.rpartition(".")[0]
    return DEFAULT


def token_for_text_and_scope(text, scope):
    """Return the pygments token for some input text and a fish token-scope path."""
    if text.isspace():
        # Here fish will return the source scope or a terminator scope for newline.
        return Text.Whitespace
    return token_for_scope(scope)


def tokenize_fish_command(code, offset):
    """Tokenize some fish code, offset in a parent string, by shelling
    out to fish_indent.

    fish_indent will output a list of csv lines: start,end,path.

    This function returns a list of (start, tok, value) tuples, as
    Pygments expects.
    """
    proc = subprocess.Popen(
        ["fish_indent", "--pygments"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        universal_newlines=False,
    )
    stdout, _ = proc.communicate(code.encode("utf-8"))
    result = []
    for line in stdout.decode("utf-8").splitlines():
        start, end, scope = line.split(",")
        start, end = int(start), int(end)
        value = code[start:end]
        tok = token_for_text_and_scope(value, scope)
        result.append((start + offset, tok, value))
    return result


class FishIndentLexer(Lexer):
    name = "FishIndentLexer"
    aliases = ["fish", "fish-docs-samples"]
    filenames = ["*.fish"]

    def get_tokens_unprocessed(self, input_text):
        """Return a list of (start, tok, value) tuples.

        start is the index into the string
        tok is the token type (as above)
        value is the string contents of the token
        """
        result = []
        if not any(s.startswith(">") for s in input_text.splitlines()):
            # No prompt, just tokenize everything.
            result = tokenize_fish_command(input_text, 0)
        else:
            # We have a prompt line.
            # Use a regexp because it will maintain string indexes for us.
            regex = re.compile(r"^(>_?\s*)?(.*\n?)", re.MULTILINE)
            for m in regex.finditer(input_text):
                if m.group(1):
                    # Prompt line; highlight via fish syntax.
                    result.append((m.start(1), Generic.Prompt, m.group(1)))
                    result.extend(tokenize_fish_command(m.group(2), m.start(2)))
                else:
                    # Non-prompt line representing output from a command.
                    result.append((m.start(2), Generic.Output, m.group(2)))
        return result
