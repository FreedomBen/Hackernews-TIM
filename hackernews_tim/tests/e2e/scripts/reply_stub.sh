#!/bin/sh
# Stub editor used by the §3.2.8 reply happy-path e2e test.
#
# `reply_editor::run_editor_for_reply` writes a scaffold with a
# `# ------ >8 ------` scissors line and execs `$VISUAL "$path"`.
# We overwrite the scaffold with a fixed body so `read_and_strip`
# lifts our text out unchanged. The lack of a scissors line in the
# overwritten file is fine — `read_and_strip`'s `take_while` stops
# at the first scissors line if present, otherwise returns the
# whole file.
printf 'Test reply body\n' > "$1"
