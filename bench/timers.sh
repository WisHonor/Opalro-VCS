#!/usr/bin/env bash
# Throwaway (branch gate/m10-timers, never merged): what each part of `opalro status` and
# `opalro goto` costs, from the timers this branch's build prints when OPALRO_TIMERS is set.
#   bash bench/timers.sh <built opalro> <work folder> [runs, 15 by default]
# Needs only bash, git, curl, tar, find, sort and awk; hyperfine, where there is one, also times
# each command whole with the timers off and on.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
bin=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
mkdir -p "$2"
work=$(cd "$2" && pwd)
runs=${3:-15}
export OPALRO_USER_NAME=Bench OPALRO_USER_EMAIL=bench@example.com
export OPALRO_CONFIG_DIR=${OPALRO_CONFIG_DIR:-$work/config}
mkdir -p "$OPALRO_CONFIG_DIR"
unset OPALRO_TIMERS

say() { printf '\n===== %s =====\n' "$*"; }

say "The code project"
[ -d "$work/code" ] || bash "$here/data.sh" code "$work/code"
p=$work/project
rm -rf "$p"
cp -a "$work/code" "$p"
cd "$p"
"$bin" start --preset custom >/dev/null
"$bin" save 'A save' --yes >/dev/null
first=$("$bin" status --json | grep -o '"id":"[0-9a-f]*"' | head -1 | cut -d'"' -f4)
changed=$(find . -path ./.opalro -prune -o -type f -name '*.c' -print | sort | sed -n 1p)
echo "/* a small change */" >>"$changed"
"$bin" save 'A small change' --yes >/dev/null
echo "$(find . -path ./.opalro -prune -o -type f -print | wc -l) files in" \
	"$(find . -path ./.opalro -prune -o -type d -print | wc -l) folders; first save $first;" \
	"the change is to $changed"
# The file cache doesn't vouch for a file changed in the last 2 seconds.
sleep 3
"$bin" status >/dev/null
"$bin" status >/dev/null

raw=$work/timers-raw.txt
: >"$raw"
# one <label> <run> <command...>: the command once with the timers printing, its lines kept.
one() {
	local label=$1 n=$2
	shift 2
	OPALRO_TIMERS=1 "$@" 2>"$work/err.txt" >/dev/null
	grep '^@T' "$work/err.txt" | awk -v l="$label" -v n="$n" '{ print l "\t" n "\t" $0 }' >>"$raw"
}

say "status with nothing changed, $runs runs with the timers printing"
for i in $(seq "$runs"); do one status "$i" "$bin" status; done
say "goto the first save and goto main again, $runs runs each with the timers printing"
for i in $(seq "$runs"); do
	one goto-first "$i" "$bin" goto "$first"
	one goto-main "$i" "$bin" goto main
done

if command -v hyperfine >/dev/null; then
	say "Each command whole, timers off and on (hyperfine, medians in ms)"
	# whole <label> <prepare> <command>: the median of 30 runs, timers off then on.
	whole() {
		local label=$1 prepare=$2 command=$3 off on
		local args=(-N --style none --warmup 3 --runs 30 --export-csv "$work/h.csv")
		[ -n "$prepare" ] && args+=(--prepare "$prepare")
		hyperfine "${args[@]}" "$command" >/dev/null
		off=$(awk -F, 'NR == 2 { printf "%.2f", $4 * 1000 }' "$work/h.csv")
		OPALRO_TIMERS=1 hyperfine "${args[@]}" "$command" >/dev/null
		on=$(awk -F, 'NR == 2 { printf "%.2f", $4 * 1000 }' "$work/h.csv")
		echo "$label: timers off $off ms, timers on $on ms"
	}
	sleep 3
	# hyperfine runs it itself, so on Windows it needs the path Windows knows.
	nbin=$(cygpath -m "$bin" 2>/dev/null || echo "$bin")
	whole status '' "$nbin status"
	whole goto-first "$nbin goto main" "$nbin goto $first"
	whole goto-main "$nbin goto $first" "$nbin goto main"
	"$bin" goto main >/dev/null
fi

say "The parts, the flushes and the renames ($runs runs; times in ms)"
awk -F'\t' '
function median(list, n,    a, i, j, t, k) {
	k = split(list, a, " ")
	for (i = 2; i <= k; i++) { t = a[i] + 0; for (j = i - 1; j >= 1 && a[j] + 0 > t; j--) a[j + 1] = a[j]; a[j + 1] = t }
	if (k == 0) return 0
	return (k % 2) ? a[(k + 1) / 2] : (a[k / 2] + a[k / 2 + 1]) / 2
}
function least(list,    a, i, k, m) { k = split(list, a, " "); m = a[1] + 0; for (i = 2; i <= k; i++) if (a[i] + 0 < m) m = a[i] + 0; return m }
function most(list,    a, i, k, m) { k = split(list, a, " "); m = a[1] + 0; for (i = 2; i <= k; i++) if (a[i] + 0 > m) m = a[i] + 0; return m }
# $1 command, $2 run, $3 "@T", $4 kind, then for a part: $5 name, $6 calls, $7 ms; for an event: $5 its number, $6 what, $7 ms.
{
	c = $1; r = $2
	if (!(c in seen)) { seen[c] = 1; commands[++nc] = c }
	if (!((c, r) in runseen)) { runseen[c, r] = 1; nr[c]++; runs[c, nr[c]] = r }
	if ($4 == "part") {
		k = c SUBSEP $5
		if (!(k in pseen)) { pseen[k] = 1; np[c]++; parts[c, np[c]] = $5 }
		ms[k, r] += $7; calls[k, r] += $6
		if ($5 ~ /^whole command/) whole[c, r] = $7
	} else {
		# Lined up across runs by what it is (and which time of it), not by its number: a run
		# may make one more or one fewer.
		o = ++occ[c, $4, r, $6]
		k = c SUBSEP $4 SUBSEP $6 SUBSEP o
		if (!(k in eseen)) { eseen[k] = 1; ne[c, $4]++; ekey[c, $4, ne[c, $4]] = k; whatof[k] = $6 (o > 1 ? " (time " o ")" : "") }
		ems[k] = ems[k] " " $7
		epos[k] = epos[k] " " $5
		count[c, $4, r]++
	}
}
END {
	for (ci = 1; ci <= nc; ci++) {
		c = commands[ci]
		wl = ""
		for (i = 1; i <= nr[c]; i++) wl = wl " " whole[c, runs[c, i]]
		w = median(wl)
		printf "\n### %s (%d runs; the whole command inside the process: median %.2f ms, %.2f to %.2f)\n\n", c, nr[c], w, least(wl), most(wl)
		print "| Part | Times in one command | Median ms | Least to most ms | Share of the whole |"
		print "|---|---|---|---|---|"
		for (i = 1; i <= np[c]; i++) {
			name = parts[c, i]; k = c SUBSEP name
			l = ""; cl = ""
			for (j = 1; j <= nr[c]; j++) { r = runs[c, j]; l = l " " (ms[k, r] + 0); cl = cl " " (calls[k, r] + 0) }
			m = median(l)
			printf "| %s | %s | %.3f | %.3f to %.3f | %.1f%% |\n", name, median(cl), m, least(l), most(l), (w > 0 ? 100 * m / w : 0)
		}
		for (e = 1; e <= 2; e++) {
			kind = (e == 1) ? "flush" : "rename"
			cl = ""
			for (j = 1; j <= nr[c]; j++) cl = cl " " (count[c, kind, runs[c, j]] + 0)
			printf "\n%s: the %s in order (per run: %d to %d):\n\n", c, (e == 1 ? "flushes" : "renames"), least(cl), most(cl)
			print "| Place (median) | What | Median ms | Least to most ms | Runs it happened in |"
			print "|---|---|---|---|---|"
			# In the order they happen: by their median place in a run.
			n = ne[c, kind]
			for (i = 1; i <= n; i++) { order[i] = ekey[c, kind, i]; pos[i] = median(epos[order[i]]) }
			for (i = 2; i <= n; i++) {
				t = order[i]; tp = pos[i]
				for (j = i - 1; j >= 1 && pos[j] > tp; j--) { order[j + 1] = order[j]; pos[j + 1] = pos[j] }
				order[j + 1] = t; pos[j + 1] = tp
			}
			for (i = 1; i <= n; i++) {
				k = order[i]
				printf "| %s | %s | %.3f | %.3f to %.3f | %d |\n", pos[i], whatof[k], median(ems[k]), least(ems[k]), most(ems[k]), split(substr(ems[k], 2), dummy, " ")
			}
		}
	}
}' "$raw"
