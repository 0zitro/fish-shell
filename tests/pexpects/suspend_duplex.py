#!/usr/bin/env python3
from pexpect_helper import SpawnedProc

sp = SpawnedProc()
sendline, sleep, expect_prompt, expect_str = (
    sp.sendline,
    sp.sleep,
    sp.expect_prompt,
    sp.expect_str,
)

expect_prompt()

# The source reads one line from the pty slave; resume supplies it from the
# terminal, and the source returns the line through its suspended stdout.
sendline(
    "set -l h (suspend --fd 5=pty "
    "'sh -c \"stty -echo <&5; read line <&5; echo got:\\$line\"')"
)
expect_prompt()
sendline("resume $h")
sleep(0.2)
sendline("hello")
expect_str("got:hello")
expect_prompt()
