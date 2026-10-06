#!/bin/sh
# Release gate: prove the consumer install of rust-repository-policy.
#
# Uses only the S2 adapter contract: the fixture's own `.alint.yml`
# extends `./.cache/rust-policy/profiles/rust-strict-v1.yml` and adds one
# `command_idempotent` rule calling `rust-repository-policy check-gaps`.
#
# Cases: 1 clean pass + 1 negative mutation per native module (8) + 1
# helper-gap defect + 1 gitignore-evasion probe, each asserting the exact
# failing rule-ID set. Helper invocations are counted via a PATH shim to
# prove exactly one `check-gaps` run per full `alint check`.
#
# Usage: check-install.sh [--fixture DIR] [--helper PATH]
#   --fixture defaults to the sibling ok/ consumer repo.
#   --helper  defaults to `rust-repository-policy` resolved from PATH.
# Exit 0 when every case matches; nonzero otherwise.
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
FIXTURE="$SCRIPT_DIR/ok"
HELPER=""

while [ $# -gt 0 ]; do
  case "$1" in
    --fixture) FIXTURE="$2"; shift 2 ;;
    --helper) HELPER="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$HELPER" ]; then
  HELPER="$(command -v rust-repository-policy || true)"
fi
if [ -z "$HELPER" ]; then
  echo "FAIL: rust-repository-policy not on PATH (pass --helper)" >&2
  exit 2
fi
command -v python3 >/dev/null 2>&1 || { echo "FAIL: python3 required" >&2; exit 2; }
command -v alint >/dev/null 2>&1 || { echo "FAIL: alint required" >&2; exit 2; }
command -v git >/dev/null 2>&1 || { echo "FAIL: git required" >&2; exit 2; }
case "$HELPER" in
  *?/*) HELPER_DIR="$(CDPATH= cd -- "$(dirname -- "$HELPER")" && pwd)"
        export PATH="$HELPER_DIR:$PATH" ;;
esac

WORK="$(mktemp -d "${TMPDIR:-/tmp}/rrpproof-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT INT TERM

FAILS=0
CASES=0

# failing rule IDs of an `alint check -f json` stdout file, one per line.
failing_ids() {
  python3 - "$1" <<'EOF'
import json, sys
data = json.load(open(sys.argv[1]))
for r in data.get("results", []):
    if not r.get("passed"):
        print(r["id"])
EOF
}

run_case() {
  # $1 = name, $2 = expected exit, $3 = expected failing IDs (space-sep), $4 = dir
  name="$1"; want_exit="$2"; want_ids="$3"; dir="$4"
  CASES=$((CASES + 1))
  code=0
  (cd "$dir" && alint check -f json --color never >"$WORK/$name.json" 2>"$WORK/$name.err") || code=$?
  got_ids=""
  if python3 -c 'import json;json.load(open("'"$WORK/$name.json"'"))' 2>/dev/null; then
    got_ids="$(failing_ids "$WORK/$name.json" | grep . | sort | tr '\n' ' ')"
  fi
  want_norm="$(echo "$want_ids" | tr ' ' '\n' | grep . | sort | tr '\n' ' ')"
  if [ "$code" = "$want_exit" ] && [ "$got_ids" = "$want_norm" ]; then
    echo "PASS $name: exit=$code failing=[$got_ids]"
  else
    FAILS=$((FAILS + 1))
    echo "FAIL $name: want exit=$want_exit failing=[$want_norm] got exit=$code failing=[$got_ids]"
    if [ -s "$WORK/$name.err" ]; then
      head -n 3 "$WORK/$name.err" | sed 's/^/     | /'
    fi
  fi
}

fresh() {
  # $1 = name -> prints a fresh copy of the fixture, initialized as a git
  # work tree like a real consumer repo (also arms the gitignore probe).
  rm -rf "$WORK/$1"
  cp -R "$FIXTURE" "$WORK/$1"
  (cd "$WORK/$1" && git init -q 2>/dev/null)
  echo "$WORK/$1"
}

echo "fixture: $FIXTURE"
echo "helper:  $HELPER"
echo "alint:   $(alint --version 2>&1 | head -n 1)"

# 0. Adapter must load: extends chain + trust gate.
if ! (cd "$FIXTURE" && alint validate-config .alint.yml -f json >"$WORK/validate.json" 2>"$WORK/validate.err"); then
  echo "BLOCKER: consumer adapter does not load:"
  sed 's/^/  | /' "$WORK/validate.err" | head -n 5
  python3 -c 'import json;print("  |", json.load(open("'"$WORK/validate.json"'"))["error"])' 2>/dev/null || true
  echo "verdict: NOT READY (config load)"
  exit 1
fi
echo "adapter loads: $(cat "$WORK/validate.json")"

# 1. Clean pass.
d="$(fresh clean)"
run_case clean 0 "" "$d"

# 2. One negative mutation per native module.
d="$(fresh layout)";      : > "$d/debug.log"
run_case layout 1 "layout-root-files" "$d"

d="$(fresh cargo)"
python3 - "$d/Cargo.toml" <<'EOF'
import sys
p = sys.argv[1]
s = open(p).read().replace('resolver = "3"', 'resolver = "2"')
assert s != open(p).read()
open(p, "w").write(s)
EOF
run_case cargo 1 "cargo-workspace-resolver" "$d"

d="$(fresh tests)";       rm -rf "$d/crates/group/app/tests"
run_case tests 1 "tests-dir-present" "$d"

d="$(fresh docs)";        : > "$d/GUIDE.md"
run_case docs 1 "docs-root-only-allowed" "$d"

d="$(fresh toolchain)"
python3 - "$d/mise.toml" <<'EOF'
import sys
p = sys.argv[1]
s = open(p).read().replace('rust = "1.98.1"', 'rust = "1.97.0"')
assert s != open(p).read()
open(p, "w").write(s)
EOF
run_case toolchain 1 "toolchain-mise-parity" "$d"

d="$(fresh ci)"
python3 - "$d/.github/workflows/ci.yml" <<'EOF'
import sys
p = sys.argv[1]
s = open(p).read().replace("runs-on: ubuntu-latest", "runs-on: ubuntu-24.04")
assert s != open(p).read()
open(p, "w").write(s)
EOF
run_case ci 1 "ci-build-runs-on" "$d"

d="$(fresh renovate)"
python3 - "$d/renovate.json" <<'EOF'
import sys
p = sys.argv[1]
s = open(p).read().replace('["config:recommended"]', "[]")
assert s != open(p).read()
open(p, "w").write(s)
EOF
run_case renovate 1 "renovate-config-schema" "$d"

d="$(fresh size)"
python3 - "$d" <<'EOF'
import sys
d = sys.argv[1]
lines = "".join(f"pub fn pad_{i}() {{}}\n" for i in range(401))
open(f"{d}/crates/group/app/src/extra.rs", "w").write(lines)
assert len(open(f"{d}/crates/group/app/src/extra.rs").readlines()) == 401
lib = f"{d}/crates/group/app/src/lib.rs"
s = open(lib).read().replace(
    "//! `\"#[test]\"` in docs and comments stays valid.",
    "//! `\"#[test]\"` in docs and comments stays valid.\nmod extra;",
)
assert s != open(lib).read()
open(lib, "w").write(s)
EOF
run_case size 1 "size-file-400" "$d"

# 3. Helper-gap defect: #[test] in a production file.
d="$(fresh helper-gap)"
printf '\n#[test]\nfn helper_gap_probe() {}\n' >> "$d/crates/group/app/src/lib.rs"
run_case helper-gap 1 "policy-gaps" "$d"
if "$HELPER" check-gaps --root "$d" >"$WORK/helper-gap.out" 2>/dev/null; then
  FAILS=$((FAILS + 1)); echo "FAIL helper-gap: check-gaps exited 0 on the defect"
else
  if grep -q "^GAP-TEST-001 crates/group/app/src/lib.rs:" "$WORK/helper-gap.out"; then
    echo "PASS helper-gap: check-gaps reports $(head -n 1 "$WORK/helper-gap.out" | cut -d' ' -f1)"
  else
    FAILS=$((FAILS + 1)); echo "FAIL helper-gap: no GAP-TEST-001 line:"; cat "$WORK/helper-gap.out"
  fi
fi

# 4. Gitignore-evasion probe: adapter sets respect_gitignore=false (the
# profile's copy does not propagate through extends), so a gitignored
# 500-line file must still fail size-file-400.
d="$(fresh gitignore)"
python3 - "$d" <<'EOF'
import sys
d = sys.argv[1]
lines = "".join(f"pub fn pad_{i}() {{}}\n" for i in range(500))
open(f"{d}/crates/group/app/src/ignored_big.rs", "w").write(lines)
lib = f"{d}/crates/group/app/src/lib.rs"
s = open(lib).read().replace(
    "//! `\"#[test]\"` in docs and comments stays valid.",
    "//! `\"#[test]\"` in docs and comments stays valid.\nmod ignored_big;",
)
assert s != open(lib).read()
open(lib, "w").write(s)
open(f"{d}/.gitignore", "w").write("crates/group/app/src/ignored_big.rs\n")
EOF
if (cd "$d" && git check-ignore -q crates/group/app/src/ignored_big.rs 2>/dev/null); then
  echo "probe armed: ignored_big.rs is git-ignored"
else
  FAILS=$((FAILS + 1)); echo "FAIL gitignore: probe file is NOT ignored (setup broken)"
fi
run_case gitignore 1 "size-file-400" "$d"

# 5. Exactly one helper invocation per full check (clean and failing).
SHIM="$WORK/shim"; mkdir -p "$SHIM"
export RRP_COUNT_FILE="$WORK/invocations.txt" RRP_REAL_HELPER="$HELPER"
cat > "$SHIM/rust-repository-policy" <<'EOF'
#!/bin/sh
echo "x" >> "$RRP_COUNT_FILE"
exec "$RRP_REAL_HELPER" "$@"
EOF
chmod +x "$SHIM/rust-repository-policy"
for target in clean helper-gap; do
  : > "$RRP_COUNT_FILE"
  code=0
  (cd "$WORK/$target" && PATH="$SHIM:$PATH" alint check --color never >/dev/null 2>&1) || code=$?
  n="$(wc -l < "$RRP_COUNT_FILE" | tr -d ' ')"
  CASES=$((CASES + 1))
  if [ "$n" = "1" ]; then
    echo "PASS invocations-$target: helper ran exactly once (alint exit=$code)"
  else
    FAILS=$((FAILS + 1))
    echo "FAIL invocations-$target: helper ran $n times (want 1, alint exit=$code)"
  fi
done

echo "cases: $CASES failures: $FAILS"
if [ "$FAILS" -gt 0 ]; then echo "verdict: NOT READY"; exit 1; fi
echo "verdict: READY"
