#!/usr/bin/env bash
# Benchmark coldluau against darklua on synthetic Rojo shaped projects
#
# coldluau does its full job (alias expansion + require rewriting to native
# string requires), darklua gets the LIGHTEST config it supports (no rules,
# retain_lines generator), so the comparison favors darklua, it only has to
# parse and reprint what coldluau parses, resolves, and rewrites
#
# Scenarios
#   cold      first build, nothing cached
#   warm      nothing changed since the last build
#   one edit  a single file touched, everything else cached
#   check     validation only, this one parses every file
#   1 big     one large module instead of many small ones
#
# Usage  scripts/bench.sh [file counts...]   defaults to 3000 5000
# Env    COLDLUAU=path DARKLUA=path RUNS=n
set -euo pipefail

SIZES=("${@:-3000 5000}")
[ $# -eq 0 ] && SIZES=(3000 5000)
RUNS="${RUNS:-5}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
COLDLUAU="${COLDLUAU:-$ROOT/target/release/coldluau}"
DARKLUA="${DARKLUA:-darklua}"

if [ ! -x "$COLDLUAU" ]; then
    echo "building coldluau (release)..."
    (cd "$ROOT" && cargo build --release)
fi
HAVE_DARKLUA=1
command -v "$DARKLUA" >/dev/null || {
    echo "note: darklua not found, running coldluau only (set DARKLUA= to compare)" >&2
    HAVE_DARKLUA=0
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

scaffold() { # scaffold <files> <body lines per file>
    python3 - "$1" "$2" <<'PY'
import os, sys
files, lines = int(sys.argv[1]), int(sys.argv[2])
dirs = max(1, files // 50)
os.makedirs("src", exist_ok=True)
body = "".join(f"    t[{i}] = i * 2\n" for i in range(lines))
n = 0
for d in range(dirs):
    os.makedirs(f"src/mod{d}", exist_ok=True)
    with open(f"src/mod{d}/init.luau", "w") as f:
        f.write("return {\n" + "".join(f'  m{i} = require("@self/sub{i}"),\n' for i in range(5)) + "}\n")
    n += 1
    i = 0
    while n < files and i < 49:
        with open(f"src/mod{d}/sub{i}.luau", "w") as f:
            dep = f'local dep = require("./sub{(i + 1) % 49}")\n' if i != 48 else "local dep = nil\n"
            f.write("--!strict\n-- header comment\n" + dep
                    + 'local pkg = require("@pkg/signal")\n'
                    + "local t = {}\nlocal function fill()\n" + body + "end\nfill()\nreturn { t, dep, pkg }\n")
        n += 1
        i += 1
    if n >= files:
        break
os.makedirs("Packages", exist_ok=True)
open("Packages/signal.luau", "w").write("return {}\n")
PY
    cat > default.project.json <<'EOF'
{
  "name": "bench",
  "tree": {
    "$className": "DataModel",
    "ReplicatedStorage": {
      "app": { "$path": "src" },
      "Packages": { "$path": "Packages" }
    }
  }
}
EOF
    cat > coldluau.toml <<'EOF'
[aliases]
pkg = "@game/ReplicatedStorage/Packages"
EOF
    cat > darklua.json <<'EOF'
{ "rules": [], "generator": "retain_lines" }
EOF
}

MEAN=0
bench() { # bench <setup> <cmd...>, sets MEAN in ms
    local setup="$1"
    shift
    "$setup"
    "$@" >/dev/null 2>&1 || {
        echo "command failed:" "$@" >&2
        exit 1
    }
    local total=0 start end ms
    for _ in $(seq "$RUNS"); do
        "$setup"
        start=$(date +%s%N)
        "$@" >/dev/null 2>&1
        end=$(date +%s%N)
        ms=$(((end - start) / 1000000))
        total=$((total + ms))
    done
    MEAN=$((total / RUNS))
}

ratio() { # ratio <slow> <fast>
    if [ "$2" -le 0 ] || [ "$1" -le 0 ]; then
        echo "-"
        return
    fi
    local r=$(($1 * 10 / $2))
    echo "$((r / 10)).$((r % 10))x"
}

drop_cache() { rm -rf .coldluau dist; }
keep_cache() { :; }
touch_one() {
    # no pipe to head here, it would SIGPIPE find and pipefail would abort
    local f
    f="$(find src -name '*.luau' -print -quit)"
    [ -n "$f" ] && touch "$f"
    return 0
}

ROWS=()
run_scenarios() { # run_scenarios <label>
    local label="$1"
    bench drop_cache "$COLDLUAU" process
    local cold=$MEAN
    bench keep_cache "$COLDLUAU" process
    local warm=$MEAN
    bench touch_one "$COLDLUAU" process
    local one=$MEAN
    bench keep_cache "$COLDLUAU" check
    local check=$MEAN

    local dark="-" speed="-"
    if [ "$HAVE_DARKLUA" = 1 ]; then
        bench keep_cache "$DARKLUA" process --config darklua.json src dist-darklua
        speed="$(ratio "$MEAN" "$cold")"
        dark="$MEAN ms"
    fi
    ROWS+=("$label|${cold} ms|${warm} ms|${one} ms|${check} ms|${dark}|$speed")
}

for size in "${SIZES[@]}"; do
    dir="$WORK/p$size"
    mkdir -p "$dir"
    cd "$dir"
    echo "benchmarking $size files, $RUNS runs each..." >&2
    scaffold "$size" 20
    run_scenarios "$size"
done

echo "benchmarking one large file..." >&2
dir="$WORK/big"
mkdir -p "$dir"
cd "$dir"
scaffold 2 5
python3 - <<'PY'
# one module of roughly a megabyte, the single file parser throughput case
lines = ['--!strict', 'local pkg = require("@pkg/signal")', 'local M = {}']
for i in range(20000):
    lines.append(f'function M.fn{i}(a: number, b: string?): number')
    lines.append(f'    local t = {{ id = {i}, name = "item{i}", flag = a > {i} }}')
    lines.append(f'    return if t.flag then a + {i} else #(b or "") * 2')
    lines.append('end')
lines.append('return { M, pkg }')
open("src/mod0/sub0.luau", "w").write("\n".join(lines) + "\n")
PY
echo "  big module is $(wc -c < src/mod0/sub0.luau) bytes" >&2
run_scenarios "1 big"

echo
echo "coldluau does full require rewriting, darklua runs no rules and only parses and prints"
echo "check parses every file, darklua has no cache so warm and one edit are coldluau only"
echo
printf "| %-6s | %-8s | %-8s | %-8s | %-8s | %-9s | %-7s |\n" \
    "Files" "cold" "warm" "one edit" "check" "darklua" "speedup"
printf "|%s|%s|%s|%s|%s|%s|%s|\n" \
    "-------:" "---------:" "---------:" "---------:" "---------:" "----------:" "--------:"
for row in "${ROWS[@]}"; do
    IFS='|' read -r f c w o ch d s <<<"$row"
    printf "| %6s | %8s | %8s | %8s | %8s | %9s | %7s |\n" "$f" "$c" "$w" "$o" "$ch" "$d" "$s"
done
