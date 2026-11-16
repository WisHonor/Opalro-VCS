#!/bin/bash
# The "try it" steps of docs/releases/v0.2.0-before-you-publish.md §2, as a script.
# Run: bash redteam/sandbox.sh run bash redteam/attacks23/try_pages.sh
set -u
O=/work/target/debug/opalro
T=$(mktemp -d)
export OPALRO_CONFIG_DIR=$T/config OPALRO_USER_NAME="Ada" OPALRO_USER_EMAIL="ada@example.com"
mkdir -p $T/p && cd $T/p
s() { printf '%s\r\n' "$2" > "$1"; }
r() { echo "\$ opalro $*"; $O "$@"; echo "[exit $?]"; }
r start --preset code
s todo.txt "buy milk"
r save "First list"
r line new menu
s todo.txt "buy milk and bread"
r save "Bread"
r goto main
s todo.txt "buy milk and eggs"
r save "Eggs"
r combine menu
r status
cat todo.txt
s todo.txt "buy milk, eggs and bread"
r resolve
r status
r history
r who todo.txt
r tag v1.0 "Tried v0.2"
r tag
r line new side
r goto main
s notes.txt "one more"
r save --amend
r save "One more"
for i in 1 2 3 4 5 6; do r undo; done
r undo --shelve
r history
r line
r status
r redo
r status
r unshelf
r status
cat todo.txt
