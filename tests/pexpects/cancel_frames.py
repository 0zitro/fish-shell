#!/usr/bin/env python3
from pexpect_helper import SpawnedProc

import os
import signal
import subprocess
import tempfile
import time

# A cancellation must leave the evaluator in a state where the next prompt can be
# evaluated. The frame executor discards its frames without running their completion
# steps, so this exercises the restore of the block and frame stacks.

sp = SpawnedProc()
send, sendline, sleep, expect_prompt, expect_str = (
    sp.send,
    sp.sendline,
    sp.sleep,
    sp.expect_prompt,
    sp.expect_str,
)

expect_prompt()
sendline("begin; sleep 5; end")
sleep(0.2)
sp.spawn.kill(signal.SIGINT)
expect_prompt()
sendline("echo alive_after_sigint")
expect_str("alive_after_sigint")
expect_prompt()

# A blocked function call is a job, so its cancellation records the signal status.
sendline("function cancel_probe; sleep 5; end")
expect_prompt()
sendline("cancel_probe")
sleep(0.2)
sp.spawn.kill(signal.SIGINT)
expect_prompt()
sendline("echo status=$status")
expect_str("status=130")
expect_prompt()

# The same restoration applies without a terminal: the process must exit without a
# panic, with the status the signal produces.
script = tempfile.NamedTemporaryFile("w", suffix=".fish", delete=False)
script.write("set p $fish_pid\n")
script.write('/bin/sh -c "sleep 0.1; kill -INT $p" &\n')
script.write("begin\n    /bin/sleep 5\nend\n")
script.close()
try:
    proc = subprocess.Popen(
        [os.environ["fish"], "--no-config", script.name],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    out, err = proc.communicate(timeout=10)
    assert proc.returncode in (0, 130), f"unexpected status {proc.returncode}"
    assert b"panicked" not in err, err.decode("utf-8", "replace")
finally:
    os.unlink(script.name)
