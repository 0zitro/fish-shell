# localization: skip(private)
# Allow %n job expansion to be used with fg/bg/wait
# `jobs` is the only one that natively supports job expansion
function __fish_expand_pid_args
    for arg in $argv
        if string match -qr '^%\d+$' -- $arg
            set -l pids (jobs -p $arg)
            or return 1
            if set -q pids[1]
                printf "%s\n" $pids
            else
                # A job whose stages are tasks has no process identifier; the builtin resolves
                # the job spec itself.
                printf "%s\n" $arg
            end
        else
            printf "%s\n" $arg
        end
    end
end
