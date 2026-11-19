#!/usr/bin/env python3
# The kind a piece of text is colours it on the interactive command line, so a variable
# that names a kind reaches the text the kind is made of.
import os

from pexpect_helper import SpawnedProc

env = os.environ.copy()
env["TERM"] = "xterm"

sp = SpawnedProc(env=env)
send, sendline, sleep, expect_prompt, expect_re, expect_str = (
    sp.send,
    sp.sendline,
    sp.sleep,
    sp.expect_prompt,
    sp.expect_re,
    sp.expect_str,
)
expect_prompt()

# One kind at a time, each with a colour no other kind in the line uses.
sendline("set -g fish_color_keyword_if red")
expect_prompt()
sendline("set -g fish_color_option_long blue")
expect_prompt()
sendline("set -g fish_color_comment green")
expect_prompt()

send("if true; echo --colour=always; end # done")
sleep(0.2)  # wait for the reader to repaint
expect_str("\x1b[31mif")
expect_str("\x1b[34m--colour")
expect_str("\x1b[32m# done")
sendline("")
expect_prompt()
