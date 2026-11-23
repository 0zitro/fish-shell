# localization: tier1
function kill
    set -l args (__fish_expand_pid_args $argv)
    and builtin kill $args
end
