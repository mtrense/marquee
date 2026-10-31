#!/bin/sh
# A fake diagram renderer for tests/diagram.rs. It reads the diagram source on stdin and
# behaves as its first argument says. A second argument names a file that gets one line
# per run, so tests can count how often the renderer ran.
mode=$1
if [ -n "$2" ]; then
    echo "$mode" >>"$2"
fi
case $mode in
svg)
    printf '<svg xmlns="http://www.w3.org/2000/svg"><text>'
    tr -d '\n'
    printf '</text></svg>\n'
    ;;
prolog)
    cat >/dev/null
    printf '<?xml version="1.0" encoding="UTF-8"?>\n'
    printf '<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">\n'
    printf '<!-- generated -->\n<svg xmlns="http://www.w3.org/2000/svg"></svg>\n'
    ;;
png)
    cat >/dev/null
    printf '\211PNG\r\n\032\nfake'
    ;;
text)
    printf '+--+\n'
    cat
    printf '+--+\n'
    ;;
fail)
    cat >/dev/null
    echo 'parse error on line 1' >&2
    exit 2
    ;;
slow)
    exec sleep 10
    ;;
garbage)
    cat >/dev/null
    echo 'not a picture'
    ;;
esac
