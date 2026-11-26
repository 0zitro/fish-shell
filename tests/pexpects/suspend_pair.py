#!/usr/bin/env python3
import os
import socket
import subprocess

source, record = socket.socketpair()
handed = os.dup(source.fileno())
recorded = os.dup(record.fileno())
source.close()
record.close()

script = (
    f"set -l h (suspend --fd 5=pair:{handed}:{recorded} 'sh -c \"cat <&5\"'); "
    "printf 'hello\\n' | resume $h"
)
try:
    process = subprocess.Popen(
        [os.environ["fish"], "--no-config", "-c", script],
        pass_fds=(handed, recorded),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    os.close(handed)
    os.close(recorded)
    stdout, stderr = process.communicate(timeout=5)
except subprocess.TimeoutExpired:
    process.kill()
    stdout, stderr = process.communicate()
    print("stdout:", stdout)
    print("stderr:", stderr)
    raise

assert process.returncode == 0, stderr
assert stdout.strip() == "hello", stdout
