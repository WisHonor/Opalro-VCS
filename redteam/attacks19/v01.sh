#!/bin/sh
# RT20: v0.1 reading a project this build made, then this build on what v0.1 left.
# Run: bash redteam/sandbox.sh run sh redteam/attacks19/v01.sh
NEW=/work/target/debug/opalro
OLD=/work/v0.1-src/target/debug/opalro
T=$(mktemp -d)
export OPALRO_USER_NAME=Test OPALRO_USER_EMAIL=t@example.com OPALRO_CONFIG_DIR=$T/cfg HOME=$T/cfg
mkdir -p $T/cfg $T/p && cd $T/p
r() { echo "\$ $*"; "$@"; echo "[exit $?]"; }
r $NEW start --preset custom >/dev/null
printf '1\n2\n3\n4\n5\n6\n7\n8\n' > a.txt; echo moved > m.txt
r $NEW save A
r $NEW line new side
printf 'one\n2\n3\n4\n5\n6\n7\n8\n' > a.txt; mv m.txt n.txt
r $NEW save S
r $NEW goto main
printf '1\n2\n3\n4\n5\n6\n7\neight\n' > a.txt
r $NEW save M
r $NEW combine side
r $NEW show --json | head -c 600; echo
echo "=== v0.1 ==="
r $OLD history
r $OLD show
r $OLD status
r $OLD verify
r $OLD goto side
r $OLD goto main
r $OLD undo --list
r $OLD undo
r $OLD undo
r $OLD redo
r $OLD tidy
r $OLD verify
ls; cat a.txt
echo "=== this build again ==="
r $NEW verify
r $NEW status
r $NEW history
r $NEW undo --list
r $NEW redo
r $NEW redo
cat a.txt; ls
r $NEW verify
echo "=== v0.1 undoes the combine itself, saves, this build combines again ==="
r $OLD undo
r $OLD undo
r $OLD status
cat a.txt
r $OLD redo
r $OLD redo
r $OLD goto main
echo nine >> a.txt
r $OLD save "after combine, by v0.1"
r $OLD line delete side --yes
r $NEW line new side2
echo x > x.txt; r $NEW save X
r $NEW goto main
r $NEW combine side2
r $OLD history
r $OLD verify
r $NEW verify
