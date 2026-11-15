#!/usr/bin/env python3
"""What the `conflicts` feature costs `status` and `goto` on a project of 100,003 files.

    python bench/conflicts.py <opalro> <work-folder>         every row, 10 runs each
    python bench/conflicts.py <opalro> <work-folder> pair    `status` alone, 36 runs each

The first form makes the project in <work-folder>/proj, which must not exist yet; the second
runs on the project the first one left. Times are whole commands, in milliseconds.
"""
import io
import json
import os
import statistics
import subprocess
import sys
import time

opalro = os.path.abspath(sys.argv[1])
work = os.path.abspath(sys.argv[2])
pair_only = sys.argv[3:] == ["pair"]
proj = os.path.join(work, "proj")
env = dict(
    os.environ,
    OPALRO_CONFIG_DIR=os.path.join(work, "config"),
    OPALRO_USER_NAME="Ada",
    OPALRO_USER_EMAIL="ada@example.com",
    NO_COLOR="1",
)
# Three files seven folders down: the ones both lines change.
DEEP = [os.path.join("d05%d" % i, "s5", "x", "y", "z", "w", "deep.txt") for i in range(3)]
FORMAT = os.path.join(proj, ".opalro", "format")
results = {}


def run(*args):
    r = subprocess.run(
        [opalro, *args], cwd=proj, env=env, capture_output=True, text=True, encoding="utf-8"
    )
    if r.returncode != 0:
        sys.exit("opalro %s ended with %d\n%s\n%s" % (" ".join(args), r.returncode, r.stdout, r.stderr))
    return r.stdout


def timed(name, *args):
    began = time.perf_counter()
    r = subprocess.run(
        [opalro, *args], cwd=proj, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
    )
    took = (time.perf_counter() - began) * 1000.0
    if r.returncode != 0:
        sys.exit("opalro %s ended with %d\n%s" % (" ".join(args), r.returncode, r.stderr.decode("utf-8", "replace")))
    results.setdefault(name, []).append(took)


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with io.open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def deep_text(who):
    lines = ["deep line %d\n" % n for n in range(8)]
    if who:
        lines[3] = "%s changed this line\n" % who
    return "".join(lines)


def save_id(message):
    for save in json.loads(run("history", "--json"))["saves"]:
        if save.get("message") == message:
            return save["id"]
    sys.exit("no save says %r" % message)


def status(name, runs):
    time.sleep(3)  # the cache doesn't vouch for a file changed in the last 2 seconds
    run("status")
    for _ in range(runs):
        timed(name + ": status", "status")


def goto(name, older, newer, runs):
    for _ in range(runs):
        timed(name + ": goto back", "goto", older)
        timed(name + ": goto forward", "goto", newer)


def make_project():
    for d in range(100):
        for s in range(10):
            folder = os.path.join(proj, "d%03d" % d, "s%d" % s)
            os.makedirs(folder)
            for f in range(100):
                text = "".join("d%03d s%d f%02d line %d\n" % (d, s, f, n) for n in range(8))
                write(os.path.join(folder, "f%02d.txt" % f), text)
    for rel in DEEP:
        write(os.path.join(proj, rel), deep_text(None))
    os.makedirs(env["OPALRO_CONFIG_DIR"], exist_ok=True)
    run("start", "--preset", "custom")
    run("save", "first", "--yes")
    for rel in DEEP:
        write(os.path.join(proj, rel), deep_text("main"))
    run("save", "second", "--yes")


if not pair_only:
    if os.path.exists(proj):
        sys.exit("%s is already there" % proj)
    make_project()

first, second = save_id("first"), save_id("second")
with io.open(FORMAT, encoding="utf-8", newline="") as f:
    with_feature = f.read()
# A project from Opalro 0.1 has the same file without this line.
without_feature = with_feature.replace("feature conflicts\n", "")
if without_feature == with_feature:
    sys.exit("the project doesn't have the conflicts feature")
kinds = [("without the feature", without_feature), ("with the feature", with_feature)]

try:
    if pair_only:
        run("goto", second)
        time.sleep(3)
        for r in range(12):
            for name, text in kinds if r % 2 == 0 else kinds[::-1]:
                write(FORMAT, text)
                run("status")
                for _ in range(3):
                    timed(name + ": status", "status")
    else:
        for _ in range(5):
            for name, text in kinds:
                write(FORMAT, text)
                status(name, 2)
                goto(name, first, "main", 2)
finally:
    write(FORMAT, with_feature)

if pair_only:
    run("goto", "main")
else:
    # Both lines change the same line of the same three files, so the combine keeps three conflicts.
    run("goto", first)
    run("line", "new", "side")
    for rel in DEEP:
        write(os.path.join(proj, rel), deep_text("side"))
    run("save", "side", "--yes")
    run("goto", "main")
    print(run("combine", "side").splitlines()[0])
    kept = len(json.loads(run("status", "--json"))["conflicts"])
    if kept != 3:
        sys.exit("expected 3 conflicts, status lists %d" % kept)
    status("3 conflicts", 10)
    goto("3 conflicts", second, "main", 10)
    # The plain project once more, to see whether the machine drifted meanwhile.
    run("goto", second)
    status("plain again", 10)
    goto("plain again", first, second, 10)
    run("goto", "main")

print("| what | runs | median | least | most |")
print("|---|---|---|---|---|")
for name, ms in results.items():
    print("| %s | %d | %.1f | %.1f | %.1f |" % (name, len(ms), statistics.median(ms), min(ms), max(ms)))
