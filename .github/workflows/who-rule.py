"""Times `opalro who --json` on each file with OPALRO_WHO_RULE unset and set to `git`, in turns,
then prints the medians, each rule's counts, and how many lines name another save.

    python3 who-rule.py <opalro> <save> <runs> <file>...

Run it in the project. Any other value of the variable keeps Opalro's rule and prints its counts.
"""

import json
import os
import statistics
import subprocess
import sys
import time

opalro, save, runs, files = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4:]


def who(file, rule):
    env = {k: v for k, v in os.environ.items() if k != "OPALRO_WHO_RULE"}
    if rule:
        env["OPALRO_WHO_RULE"] = rule
    began = time.perf_counter()
    done = subprocess.run(
        [opalro, "who", file, "--save", save, "--json"], env=env, capture_output=True
    )
    took = time.perf_counter() - began
    if done.returncode != 0:
        sys.exit(f"{file}, OPALRO_WHO_RULE={rule}: exit {done.returncode}\n{done.stderr.decode()}")
    return took, done


def each_line(done):
    doc = json.loads(done.stdout)
    return [run["save"] for run in doc["runs"] for _ in range(run["lines"])], doc["coarse"]


def timed(took):
    return f"{statistics.median(took):.2f} s ({min(took):.2f} to {max(took):.2f})"


for file in files:
    took = {None: [], "git": []}
    last = {}
    for _ in range(runs):
        for rule in took:
            t, last[rule] = who(file, rule)
            took[rule].append(t)
    _, counted = who(file, "opalro")
    ours, ours_coarse = each_line(last[None])
    gits, gits_coarse = each_line(last["git"])
    assert ours and len(ours) == len(gits), file
    other = sum(a != b for a, b in zip(ours, gits))
    print(f"{file}: {len(ours)} lines; median of {runs} runs each, in turns:")
    print(f"  Opalro's rule {timed(took[None])}, Git's rule {timed(took['git'])}")
    print(f"  {counted.stderr.decode().strip()}")
    print(f"  {last['git'].stderr.decode().strip()}")
    print(f"  {other} lines name another save under Git's rule")
    print(f"  coarse: {ours_coarse} and {gits_coarse}; the same answer with the counts printed: "
          f"{counted.stdout == last[None].stdout}")
