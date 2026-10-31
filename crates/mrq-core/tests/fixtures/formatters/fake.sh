#!/bin/sh
# A fake code formatter for tests/fmt_code.rs. It reads the code on stdin and behaves as
# its first argument says; any further arguments are written before the code.
mode=$1
shift
case $mode in
upper)
    for arg in "$@"; do printf '%s\n' "$arg"; done
    tr '[:lower:]' '[:upper:]'
    ;;
fence)
    cat
    printf '%s\n' '````' '~~~~~'
    ;;
no-newline)
    cat >/dev/null
    printf 'formatted'
    ;;
fail)
    cat >/dev/null
    echo 'syntax error on line 1' >&2
    echo 'more details' >&2
    exit 3
    ;;
slow)
    exec sleep 10
    ;;
binary)
    cat >/dev/null
    printf '\377\376\n'
    ;;
esac
