#!/bin/sh
if [ "$#" -eq 1 ] && [ "$1" = "+%s%3N" ]; then
    exec /bin/date +%s000
fi
exec /mnt/sdcard/glibc/date "$@"
