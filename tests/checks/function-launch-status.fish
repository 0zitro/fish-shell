# RUN: %fish %s

# A function whose only job fails during launch executes no command, so its status
# stays at the previous value, as it does for a function that runs nothing.

function m
    echo hi > /no_such_dir_xyzzy/x
end

m
echo $status
# CHECK: 0
# CHECKERR: warning: An error occurred while redirecting file '/no_such_dir_xyzzy/x'
# CHECKERR: warning: Path '/no_such_dir_xyzzy' does not exist

m | cat
echo $pipestatus
# CHECK: 0 0
# CHECKERR: warning: An error occurred while redirecting file '/no_such_dir_xyzzy/x'
# CHECKERR: warning: Path '/no_such_dir_xyzzy' does not exist
