#!/usr/bin/env python3
from pexpect_helper import SpawnedProc

sp = SpawnedProc()
send, sendline, sleep, expect_prompt, expect_str = (
    sp.send,
    sp.sendline,
    sp.sleep,
    sp.expect_prompt,
    sp.expect_str,
)

expect_prompt()

# The command form of suspend returns the interrupted status and no handle
# when its internal source is a blocking read.
sendline("set -l h (suspend 'read -l line')")
sleep(0.2)
send("\x03")
expect_prompt()
sendline("echo suspend_status=$status")
expect_str("suspend_status=130")
expect_prompt()
sendline("count $h")
expect_str("0")
expect_prompt()
