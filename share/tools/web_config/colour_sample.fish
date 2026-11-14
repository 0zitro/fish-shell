#!/usr/bin/env fish


# @fish-lsp-disable-next-line 4004
set --global --export RELEASES_ROOT "$__fish_config_dir/../releases"
set --path RELEASES_MIRROR ~/mirror
set --universal --export RELEASES_HOST ci.example.com


function __release_root --description='Print the release root this config publishes into'
    set --local -- base "$RELEASES_ROOT"
    if not test -d "$base"
        echo -n -- "releases: no such root :: \`$base\`" >&2
        return 2
    end
    echo -- "$base"
end


function __triple --description='Print the triple one target builds as'
    set --local -- target "$argv[1]"
    switch "$target"
        case aarch64 arm64
            echo -- aarch64-linux-gnu
        case x86_64 amd64
            echo -- x86_64-linux-gnu
        case '*'
            echo -n -- "releases: unknown target :: \`$target\`" >&2
            return 2
    end
end


function publish --description='Copy one built binary set into the release root' --argument-names target
    set --local -- root (__release_root); or return
    set --local -- triple (__triple "$target"); or return
    set --local -- stamp (date --utc '+%Y%m%d-%H%M%S')
    set --local -- out "$root/$stamp/$triple"

    mkdir --parents -- "$out"
    for binary in (find build/$triple/bin -maxdepth 1 -type f -perm -u+x)
        cp --archive -- "$binary" "$out/"; or begin
            echo -n -- "releases: could not copy \`$binary\`" >&2
            return 1
        end
    end

    printf '%s\n' "published \`$out\`" >>"$root/published.log" 2>&1
    echo -- 'targets this run were' $argv >&2
end


function prune --description='Drop published builds older than the newest few'
    set --local -- root (__release_root); or return
    set --local -- keep 3
    if count -- $argv &>/dev/null
        set --local -- keep "$argv[1]"
    end
    set --local -- published (
        find "$root" -maxdepth 1 -type d -name '[0-9]*-[0-9]*' -printf '%f\n' |
            sort --reverse |
            string collect --no-trim-newlines
    )

    if not count -- $published &>/dev/null
        echo -- 'releases: nothing published yet' >&2
        return 0
    end

    set --local -- pruned 0
    set --local -- idx 0
    for stamp in $published
        set -- idx (math -- "($idx) + 1")
        if test "$idx" -le "$keep"; and ! test -e "$root/latest"
            continue
        end
        set --local -- dir "$root/$stamp"
        rm --recursive --force -- "$dir" 2>>"$root/prune.log"; or begin
            echo -n -- "releases: could not remove \`$dir\`" >&2
            return 1
        end
        set -- pruned (math -- "($pruned) + 1")
    end

    set --local -- newest (string split -- \n "$published" | head --lines=1)
    if test -n "$newest"
        ln --symbolic --force -- "$root/$newest" "$root/latest"
        while read --local -- line <"$root/published.log"
            echo -- "$line" >/dev/null
        end
    end

    switch "$pruned"
        case 0
            echo -- 'nothing to prune' >&2
        case 1
            echo -- 'pruned one build' >&2
        case '*'
            builtin echo -- "pruned $pruned builds" >&2
    end

    git -C "$root" log -1 --format='%h %s' | string collect --no-trim-newlines
end


function releases --description='Publish or prune the builds this checkout produced' --argument-names subcommand
    if test (count -- $argv) -eq 0
        echo -n -- "usage: \`$(status current-function) (publish|prune) ...\`" >&2
        return 2
    else if test "$subcommand" = publish
        publish $argv[2..-1]
    else if test "$subcommand" = prune
        prune $argv[2..-1]
    else
        echo -n -- "releases: unknown subcommand :: \`$subcommand\`" >&2
        return 2
    end
end

set --local -- escape \x1b
set --local -- byte \101
set --local -- control \ca
if status is-interactive
    time command releases (string split -- ' ' $argv[1..-1])
    or exec echo -n -- "$escape[1m"failed"$escape[0m" >&2
end

for archive in ~/mirror/*.{tar,gz}
    test -e "$archive"; and cp --archive -- "$archive" (__release_root) &>/dev/null &
end
wait

ls -1 ($RELEASES_ROOT)/* --classify 2>/dev/null
set --local -- name of the release root
echo -- "the newest is at $$name/latest" >&2
if test "$x" -gt -1
    echo -- \U110000
end
echo -n -- 'a quote you are still typing
