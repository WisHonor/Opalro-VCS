# Throwaway, gate branch only: where `opalro who` and `git blame` name different commits, checks
# each answer against Git and sorts the lines into classes.
# Usage: analyze.py <git clone> <tip> <who.tsv> <out dir>
import bisect
import collections
import os
import subprocess
import sys

REPO, TIP, WHO_TSV, OUT = sys.argv[1:5]
FILES = os.environ.get("WHODIFF_FILES", "Makefile,diff.c,git-compat-util.h,README.md,t/t8002-blame.sh").split(",")
NONE_LIMIT = 40


def git(*args):
    return subprocess.run(["git", *args], cwd=REPO, check=True, capture_output=True).stdout


class Cat:
    def __init__(self):
        self.p = subprocess.Popen(["git", "cat-file", "--batch"], cwd=REPO,
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE)

    def get(self, spec):
        self.p.stdin.write(spec.encode() + b"\n")
        self.p.stdin.flush()
        head = self.p.stdout.readline().split()
        if head[-1] == b"missing" or len(head) < 3:
            return None, None
        size = int(head[2])
        data = self.p.stdout.read(size)
        self.p.stdout.read(1)
        return head[1].decode(), data


cat = Cat()
_blobs, _commits, _anc = {}, {}, {}


def lines(commit, path):
    """The file's lines as (text without ending, raw ending) or None if no text file there."""
    key = (commit, path)
    if key not in _blobs:
        kind, data = cat.get(f"{commit}:{path}")
        if kind != "blob":
            _blobs[key] = None
        else:
            out = []
            parts = data.split(b"\n")
            for i, part in enumerate(parts):
                last = i == len(parts) - 1
                if last and part == b"":
                    break
                end = b"" if last else b"\n"
                if part.endswith(b"\r"):
                    part, end = part[:-1], b"\r" + end
                out.append((part, end))
            _blobs[key] = out
    return _blobs[key]


def texts(commit, path):
    v = lines(commit, path)
    return None if v is None else [t for t, _ in v]


def commit_info(c):
    if c not in _commits:
        _, data = cat.get(c)
        head, _, msg = data.partition(b"\n\n")
        parents, date = [], ""
        for h in head.split(b"\n"):
            if h.startswith(b"parent "):
                parents.append(h[7:].decode())
            elif h.startswith(b"committer "):
                date = h.rsplit(b" ", 2)[-2].decode()
        subject = msg.split(b"\n", 1)[0].decode("utf-8", "replace")
        day = git("show", "-s", "--format=%ad", "--date=short", c).decode().strip()
        _commits[c] = {"parents": parents, "date": day, "subject": subject}
    return _commits[c]


def parents(c):
    return commit_info(c)["parents"]


def is_anc(a, b):
    if (a, b) not in _anc:
        r = subprocess.run(["git", "merge-base", "--is-ancestor", a, b], cwd=REPO)
        _anc[(a, b)] = r.returncode == 0
    return _anc[(a, b)]


def check1(c, path, t):
    v = texts(c, path)
    if v is None:
        return "NO FILE at its commit"
    n = v.count(t)
    if n == 0:
        return "WRONG: its version does not hold the text"
    if n > 1:
        return "uncheckable (text repeats at its commit)"
    with_file = [p for p in parents(c) if texts(p, path) is not None]
    holders = [p for p in with_file if t in texts(p, path)]
    if not holders:
        return "right"
    if len(holders) == len(with_file):
        return "WRONG: every parent with the file holds the text"
    return "WRONG: a parent holds the text (not every one)"


def verdict(s):
    return "right" if s == "right" else ("uncheckable" if s.startswith("uncheckable") else "wrong")


fp_chain = git("rev-list", "--first-parent", TIP).decode().split()  # newest first
fp_set = set(fp_chain)
fp_old_first = list(reversed(fp_chain))


def joining_merge(x):
    """The first-parent-chain commit that first brings x in (None if x is on the chain)."""
    if x in fp_set:
        return None
    lo, hi = 0, len(fp_old_first) - 1
    while lo < hi:
        mid = (lo + hi) // 2
        if is_anc(x, fp_old_first[mid]):
            hi = mid
        else:
            lo = mid + 1
    return fp_old_first[lo]


def describe(c):
    i = commit_info(c)
    return f"{c[:12]} {i['date']} parents {' '.join(p[:12] for p in i['parents']) or '-'} \"{i['subject']}\""


def around(c, path, t, anchor=None, n=3):
    v = texts(c, path)
    if v is None:
        return ["      (no file here)"]
    idx = v.index(t) if t in v else (v.index(anchor) if anchor is not None and anchor in v else None)
    if idx is None:
        return ["      (neither the text nor the line above it is here)"]
    out = []
    for k in range(max(0, idx - n), min(len(v), idx + n + 1)):
        mark = ">>" if v[k] == t else "  "
        out.append(f"    {mark}{k + 1:5} {v[k].decode('utf-8', 'replace')}")
    return out


def blame(path):
    raw = git("blame", "--line-porcelain", TIP, "--", path)
    out, cur = [], None
    for ln in raw.split(b"\n"):
        if cur is None:
            if not ln:
                continue
            f = ln.split(b" ")
            cur = {"commit": f[0].decode(), "orig": int(f[1]), "previous": None, "filename": None}
        elif ln.startswith(b"\t"):
            out.append(cur)
            cur = None
        elif ln.startswith(b"filename "):
            cur["filename"] = ln[9:].decode()
        elif ln.startswith(b"previous "):
            sha, p = ln[9:].split(b" ", 1)
            cur["previous"] = (sha.decode(), p.decode())
    return out


who = collections.defaultdict(dict)
with open(WHO_TSV, encoding="utf-8") as fh:
    for row in fh:
        f, n, c, was = row.rstrip("\n").split("\t")
        who[f][int(n)] = (c, was or f)

report, none_out, merge_out, summary = [], [], [], {}
CLASSES = ["rename", "line ending", "repeats", "merge", "none of these"]
for F in FILES:
    tip = texts(TIP, F)
    bl = blame(F)
    w = who[F]
    assert len(tip) == len(bl) == len(w), (F, len(tip), len(bl), len(w))
    s = {"lines": len(tip), "differ": 0, "classes": collections.Counter(),
         "who_check": collections.Counter(), "blame_check": collections.Counter(),
         "who_wrong": [], "blame_wrong": [], "rename_w": collections.Counter(),
         "merge_sides": collections.Counter(), "repeats_who_possible": 0,
         "repeats_blame_possible": 0, "none": 0}
    for i, t in enumerate(tip, start=1):
        W, Wpath = w[i]
        b = bl[i - 1]
        B, Bpath = b["commit"], b["filename"]
        if W == B:
            continue
        s["differ"] += 1
        cw, cb = check1(W, Wpath, t), check1(B, Bpath, t)
        s["who_check"][verdict(cw)] += 1
        s["blame_check"][verdict(cb)] += 1
        if verdict(cw) == "wrong":
            s["who_wrong"].append((i, t, W, Wpath, cw, B, cb))
        if verdict(cb) == "wrong":
            s["blame_wrong"].append((i, t, W, B, Bpath, cb))
        # Classes, the first that fits.
        cls = None
        if Bpath != F:
            cls = "rename"
            brought = all(texts(p, F) is None for p in parents(W))
            s["rename_w"]["W brought the file to its path" if brought else "W is later"] += 1
        if cls is None:
            vb = lines(B, Bpath)
            prev = b["previous"] or ((parents(B)[0], Bpath) if parents(B) else None)
            pv = lines(*prev) if prev else None
            if vb and pv is not None:
                rawb = vb[b["orig"] - 1]
                if rawb[0] == t and t in [x for x, _ in pv] and rawb not in pv:
                    cls = "line ending"
            if cls is None and parents(W):
                vw, pw = lines(W, Wpath), lines(parents(W)[0], Wpath)
                if vw and pw and [x for x, _ in vw] == [x for x, _ in pw] and vw != pw:
                    cls = "line ending"
        if cls is None:
            tw, tb = texts(W, Wpath) or [], texts(B, Bpath) or []
            if tip.count(t) > 1 or tw.count(t) > 1 or tb.count(t) > 1:
                cls = "repeats"
                for who_side, C, P in (("who", W, Wpath), ("blame", B, Bpath)):
                    tc = texts(C, P) or []
                    pc = texts(parents(C)[0], P) if parents(C) else None
                    if tc.count(t) >= 1 and (pc is None or pc.count(t) < tc.count(t)):
                        s[f"repeats_{who_side}_possible"] += 1
        if cls is None:
            unrelated = not is_anc(W, B) and not is_anc(B, W)
            if len(parents(W)) > 1 or len(parents(B)) > 1 or unrelated:
                cls = "merge"
                s["merge_sides"][f"who on first-parent chain: {W in fp_set}; blame: {B in fp_set}; "
                                 f"W merge {len(parents(W)) > 1}; B merge {len(parents(B)) > 1}; unrelated {unrelated}"] += 1
        if cls is None:
            cls = "none of these"
            s["none"] += 1
            if s["none"] <= NONE_LIMIT:
                txt = t.decode("utf-8", "replace")
                above = tip[i - 2] if i > 1 else None
                none_out.append(f"### {F}:{i}  `{txt}`")
                none_out.append(f"- who W:   {describe(W)} (path {Wpath}); check 1: {cw}")
                none_out.append(f"- blame B: {describe(B)} (path {Bpath}); check 1: {cb}")
                none_out.append(f"- W ancestor of B: {is_anc(W, B)}; B ancestor of W: {is_anc(B, W)}")
                for label, C, P in (("W", W, Wpath), ("B", B, Bpath)):
                    none_out.append(f"  {label}'s version:")
                    none_out += around(C, P, t, above)
                    if parents(C):
                        none_out.append(f"  {label}'s first parent {parents(C)[0][:12]}:")
                        none_out += around(parents(C)[0], P, t, above)
                none_out.append("")
        s["classes"][cls] += 1
        # The merge rule: where W and B came in on different sides of a merge.
        for name, X in (("who", W), ("blame", B)):
            m = joining_merge(X)
            if m is None:
                continue
            ps = parents(m)
            held = [t in (texts(p, F) or []) for p in ps]
            if len(ps) > 1 and all(held):
                merge_out.append(f"{F}:{i} differing; {name} named {X[:12]} (second-parent side of "
                                 f"{m[:12]}); both parents of {m[:12]} hold the text at {F}; "
                                 f"other program named {(B if name == 'who' else W)[:12]} "
                                 f"(on {m[:12]}^1's side: {is_anc(B if name == 'who' else W, ps[0])})")
    # A sample of agreeing lines: does a line named on a merge's second side also sit in its first?
    agree = [i for i in range(1, len(tip) + 1) if w[i][0] == bl[i - 1]["commit"]]
    step = max(1, len(agree) // 80)
    sample = agree[::step]
    s["agree_sample"] = len(sample)
    s["agree_second_side"] = 0
    s["agree_both_hold"] = 0
    for i in sample:
        t, X = tip[i - 1], w[i][0]
        m = joining_merge(X)
        if m is None:
            continue
        s["agree_second_side"] += 1
        ps = parents(m)
        if len(ps) > 1 and t in (texts(ps[0], F) or []):
            s["agree_both_hold"] += 1
            merge_out.append(f"{F}:{i} agreeing; both named {X[:12]} on {m[:12]}'s second-parent side, "
                             f"yet {m[:12]}^1 holds the text at {F} ({(texts(ps[0], F) or []).count(t)} times; "
                             f"in the tip {tip.count(t)} times)")
    summary[F] = s

lines_out = []
lines_out.append("| file | lines | differ | " + " | ".join(CLASSES) + " |")
lines_out.append("|---|---|---|" + "---|" * len(CLASSES))
for F, s in summary.items():
    lines_out.append(f"| {F} | {s['lines']} | {s['differ']} | " +
                     " | ".join(str(s["classes"][c]) for c in CLASSES) + " |")
lines_out.append("")
lines_out.append("| file | who right | who wrong | who uncheckable | blame right | blame wrong | blame uncheckable | repeats: who possible | repeats: blame possible |")
lines_out.append("|---|---|---|---|---|---|---|---|---|")
for F, s in summary.items():
    a, b = s["who_check"], s["blame_check"]
    lines_out.append(f"| {F} | {a['right']} | {a['wrong']} | {a['uncheckable']} | {b['right']} | "
                     f"{b['wrong']} | {b['uncheckable']} | {s['repeats_who_possible']} of {s['classes']['repeats']} | "
                     f"{s['repeats_blame_possible']} of {s['classes']['repeats']} |")
lines_out.append("")
for F, s in summary.items():
    lines_out.append(f"{F}: rename lines, where who stopped: {dict(s['rename_w'])}")
    for k, v in s["merge_sides"].items():
        lines_out.append(f"{F}: merge class: {k}: {v}")
    lines_out.append(f"{F}: agreeing sample {s['agree_sample']}, named on a merge's second side "
                     f"{s['agree_second_side']}, of those with the first parent also holding the text "
                     f"{s['agree_both_hold']}")
lines_out.append("")
lines_out.append("## who's answers that check 1 calls wrong")
for F, s in summary.items():
    for (i, t, W, Wpath, cw, B, cb) in s["who_wrong"]:
        lines_out.append(f"### {F}:{i} `{t.decode('utf-8', 'replace')}`")
        lines_out.append(f"- who W: {describe(W)} (path {Wpath}): {cw}")
        lines_out.append(f"- blame B: {describe(B)}: {cb}")
        for p in parents(W):
            lines_out.append(f"  W's parent {p[:12]} holds it {(texts(p, Wpath) or []).count(t)} times")
        lines_out += around(W, Wpath, t)
        for p in parents(W):
            lines_out.append(f"  in W's parent {p[:12]}:")
            lines_out += around(p, Wpath, t)
lines_out.append("")
lines_out.append("## blame's answers that check 1 calls wrong")
for F, s in summary.items():
    for (i, t, W, B, Bpath, cb) in s["blame_wrong"]:
        lines_out.append(f"- {F}:{i} `{t.decode('utf-8', 'replace')}` B {describe(B)} (path {Bpath}): {cb}")

with open(f"{OUT}/summary.md", "w", encoding="utf-8", newline="\n") as fh:
    fh.write("\n".join(lines_out) + "\n")
with open(f"{OUT}/none-of-these.md", "w", encoding="utf-8", newline="\n") as fh:
    fh.write("\n".join(none_out) + "\n")
with open(f"{OUT}/merges.txt", "w", encoding="utf-8", newline="\n") as fh:
    fh.write("\n".join(merge_out) + "\n")
print("\n".join(lines_out[:40]))
print(f"none-of-these blocks: {sum(s['none'] for s in summary.values())}; merge lines: {len(merge_out)}")
