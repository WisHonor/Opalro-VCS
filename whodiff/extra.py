# Throwaway, gate branch only: a closer look at the merges and moved lines run 1 found.
# Usage: extra.py <git clone> <tip>
import subprocess
import sys

REPO, TIP = sys.argv[1:3]


def git(*args):
    r = subprocess.run(["git", *args], cwd=REPO, capture_output=True)
    return r.stdout.decode("utf-8", "replace")


def line_at(rev, f, n):
    return git("show", f"{rev}:{f}").split("\n")[n - 1]


def where(rev, f, t):
    v = git("show", f"{rev}:{f}").split("\n")
    return [i + 1 for i, x in enumerate(v) if x == t]


print(git("--version"))
# (file, tip line, merge) for lines whose text both parents of a merge hold.
MERGES = [("Makefile", 740, "bf10200871d9"), ("Makefile", 3373, "306ab352f4e9"),
          ("diff.c", 416, "cf792653ad40"), ("diff.c", 786, "30035d1d6005"),
          ("diff.c", 795, "30035d1d6005"), ("diff.c", 803, "30035d1d6005"),
          ("diff.c", 2103, "9847a524321a"), ("diff.c", 4622, "ff08c373dccf"),
          ("diff.c", 4622, "fed276e7524d"), ("diff.c", 7449, "8f5dc5a4af3f"),
          ("git-compat-util.h", 638, "ebdb4c523d5e"), ("diff.c", 1916, "d91ba8fa88a8"),
          ("diff.c", 1921, "9981c808b4b9"), ("diff.c", 1909, "0f0a57e1e3df")]
for f, n, m in MERGES:
    t = line_at(TIP, f, n)
    ps = git("rev-list", "--parents", "-n1", m).split()[1:]
    blobs = {r: git("rev-parse", f"{r}:{f}").strip()[:12] for r in [m, *ps]}
    same = [i + 1 for i, p in enumerate(ps) if blobs[p] == blobs[m]]
    print(f"== {f}:{n} `{t.strip()}` merge {m}: file same as parent(s) {same or 'none'}; "
          f"text at lines {where(m, f, t)} in the merge, "
          + "; ".join(f"parent {i + 1} {p[:12]} at {where(p, f, t)}" for i, p in enumerate(ps)))
    for k in where(m, f, t)[:1]:
        print("   git blame at the merge:", git("blame", "-s", "-l", "-L", f"{k},{k}", m, "--", f).strip()[:120])

# Commits that moved a line: the hunks of their diff holding the text.
MOVES = [("Makefile", 898, "fcb6c0760d14"), ("Makefile", 1059, "8474f2658156"),
         ("Makefile", 1159, "805d9eaf5eab"), ("Makefile", 1177, "8275905e7e9e"),
         ("Makefile", 1251, "805d9eaf5eab"), ("Makefile", 1341, "805d9eaf5eab"),
         ("Makefile", 1345, "805d9eaf5eab"), ("Makefile", 1351, "fcb6c0760d14"),
         ("diff.c", 2103, "2e5d2003b288"), ("diff.c", 4622, "a5a3878ba7a0"),
         ("diff.c", 4622, "eede7b7d110e")]
for f, n, c in MOVES:
    t = line_at(TIP, f, n)
    print(f"== {f}:{n} `{t.strip()}` in {c}: text at {where(c + '^', f, t)} before, {where(c, f, t)} after")
    hunks = git("diff", "-U1", f"{c}^", c, "--", f).split("\n@@")
    for h in hunks:
        if any(x[1:] == t for x in h.split("\n") if x[:1] in "+-"):
            print("@@" + h[:1500])
