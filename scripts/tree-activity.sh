#!/usr/bin/env bash
# tree-activity.sh — "is another agent MUTATING this working tree right now?"
#
# aegis-nv855. Fourth guard in the family, and the only one whose state is
# SHARED and LIVE rather than one session's introspection.
#
# THE SIGNAL IS NOT "two agents touched this repo". It is "two agents are
# mutating the SAME WORKING TREE without worktree isolation." Reads are fine.
# Worktree-isolated writes are fine — that is the entire point of aegis-h2rr.
# Concurrent mutation of one index/HEAD is the fault.
#
# INFORMS, NEVER BLOCKS. In the incident this comes from, two agents pulled the
# same clone 20 minutes apart, each having reasoned correctly that it was
# permitted, neither aware the other was there. Neither needed stopping; they
# needed to KNOW. A guard that blocked would have refused correct behaviour and
# been muted within a day. Exit is ALWAYS 0.
#
# THE DISCRIMINATOR, and why it is `--show-toplevel` and not the repo name:
#
#     shared checkout   toplevel=/…/bobbin           common-dir=.git
#     agent worktree    toplevel=/…/bobbin-wt/alice  common-dir=/…/bobbin/.git
#
# Two worktrees of ONE repo have DIFFERENT toplevels, so keying on the toplevel
# makes worktree-isolated work silent for free, while two agents in one checkout
# collide on the same key. Keying on the repo (or on common-dir) would fire on
# exactly the discipline aegis-iaef mandates.
#
# Symlinks are resolved (`-P`), because the case that started this reached a
# crew clone through ~/.claude/skills/* and was never git-mediated at all.
#
# Usage:
#   tree-activity.sh begin <path> [op]   # record: I am mutating this tree
#   tree-activity.sh end   <path>        # record: I am done
#   tree-activity.sh check <path>        # who ELSE is in here? (advisory text)
#   tree-activity.sh list                # everything currently registered
#   tree-activity.sh --selftest          # prove the discrimination test
set -uo pipefail

REG="${AEGIS_TREE_ACTIVITY_DIR:-${XDG_RUNTIME_DIR:-/tmp}/aegis-tree-activity}"
STALE_SECS="${AEGIS_TREE_ACTIVITY_STALE:-900}"   # 15 min; a dead session must not haunt a tree
ME="${AEGIS_TREE_ACTIVITY_ME:-${GT_ROLE:-${BD_ACTOR:-${USER:-unknown}}}}"
# Session identity. PPID is readonly in bash, so it cannot be overridden for
# tests — carry it in a variable of our own instead.
SID="${AEGIS_TREE_ACTIVITY_PID:-$PPID}"

mkdir -p "$REG" 2>/dev/null || true

# Identity of a WORKING TREE. Falls back to the resolved path for non-git dirs,
# so a symlinked skills directory is still keyed consistently.
tree_id() {
  local p="${1:-.}" top
  [ -e "$p" ] || { echo ""; return; }
  local dir; dir="$(cd -P "$(dirname "$p")" 2>/dev/null && pwd -P)" || { echo ""; return; }
  [ -d "$p" ] && dir="$(cd -P "$p" 2>/dev/null && pwd -P)"
  top="$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null)"
  [ -n "$top" ] || top="$dir"
  # resolve again: a worktree path may itself be reached through a symlink
  ( cd -P "$top" 2>/dev/null && pwd -P ) || echo "$top"
}

key_of() { printf '%s' "$1" | sed 's#[^A-Za-z0-9]#_#g'; }
now()    { date +%s; }

reap() {
  local n; n=$(now)
  for f in "$REG"/*.entry; do
    [ -e "$f" ] || continue
    local ts pid
    ts=$(sed -n '3p' "$f" 2>/dev/null); pid=$(sed -n '2p' "$f" 2>/dev/null)
    [ -n "$ts" ] || { rm -f "$f"; continue; }
    # stale by age, OR the owning process is gone — both mean nobody is there
    if [ $(( n - ts )) -ge "$STALE_SECS" ] && [ "$STALE_SECS" -le 0 ] || [ $(( n - ts )) -gt "$STALE_SECS" ] || { [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; }; then
      rm -f "$f"
    fi
  done
}

cmd_begin() {
  local tid; tid="$(tree_id "${1:-.}")"; [ -n "$tid" ] || return 0
  local op="${2:-mutate}"
  printf '%s\n%s\n%s\n%s\n%s\n' "$ME" "$SID" "$(now)" "$op" "$tid" \
    > "$REG/$(key_of "$tid")__${ME}__${SID}.entry" 2>/dev/null || true
}

cmd_end() {
  local tid; tid="$(tree_id "${1:-.}")"; [ -n "$tid" ] || return 0
  rm -f "$REG/$(key_of "$tid")__${ME}__${SID}.entry" 2>/dev/null || true
}

# The advisory. Prints ONLY when someone else is genuinely in this tree.
cmd_check() {
  reap
  local tid; tid="$(tree_id "${1:-.}")"; [ -n "$tid" ] || return 0
  local pfx="$REG/$(key_of "$tid")__" found=0 out=""
  for f in "$pfx"*.entry; do
    [ -e "$f" ] || continue
    local who pid ts op age
    who=$(sed -n '1p' "$f"); pid=$(sed -n '2p' "$f"); ts=$(sed -n '3p' "$f"); op=$(sed -n '4p' "$f")
    [ "$who" = "$ME" ] && [ "$pid" = "$SID" ] && continue   # never warn about myself
    age=$(( $(now) - ts ))
    out+="    - ${who} (pid ${pid}) — ${op}, ${age}s ago"$'\n'
    found=1
  done
  [ "$found" = 1 ] || return 0
  cat <<EOF
⚠ ANOTHER AGENT IS MUTATING THIS WORKING TREE RIGHT NOW
    tree: $tid
$out
  This is ADVISORY — you are not blocked, and you may well be right to proceed.
  You are being told because the tree cannot tell you: two agents mutating one
  index/HEAD silently clobber each other and BOTH are reported success.
  If you are about to write here: take a worktree instead —
      st worktree <repo>      (or scripts/crew-worktree.sh --repo <repo>)
  If you must share it, say so on the bead and coordinate directly.
EOF
}

cmd_list() { reap; for f in "$REG"/*.entry; do [ -e "$f" ] || continue
  printf '%-10s pid=%-7s age=%-5s %s\n' "$(sed -n '1p' "$f")" "$(sed -n '2p' "$f")" \
    "$(( $(now) - $(sed -n '3p' "$f") ))s" "$(sed -n '5p' "$f")"; done; }

# ── The acceptance, as an executable discrimination test ────────────────────
# The bead is explicit that a guard which cannot separate these is NOT READY.
selftest() {
  local tmp; tmp="$(mktemp -d)"; local pass=0 fail=0
  export AEGIS_TREE_ACTIVITY_DIR="$tmp/registry"; REG="$AEGIS_TREE_ACTIVITY_DIR"; mkdir -p "$REG"
  git init -q "$tmp/repo" 2>/dev/null
  ( cd "$tmp/repo" && git -c user.email=t@t -c user.name=t commit -q --allow-empty -m init )
  git -C "$tmp/repo" worktree add -q -b wt-alice "$tmp/repo-wt-alice" 2>/dev/null
  ln -s "$tmp/repo" "$tmp/symlink-to-repo"

  ck() { # name expect_fire path
    local got; got="$( "$0" check "$2" )"
    local fired=0; [ -n "$got" ] && fired=1
    if [ "$fired" = "$3" ]; then echo "  PASS  $1"; pass=$((pass+1))
    else echo "  FAIL  $1 (expected fire=$3 got=$fired)"; fail=$((fail+1)); fi
  }

  echo "MUST FIRE:"
  AEGIS_TREE_ACTIVITY_ME=alice AEGIS_TREE_ACTIVITY_PID=$$ "$0" begin "$tmp/repo" "git pull"
  AEGIS_TREE_ACTIVITY_ME=bob   ck "two agents mutating ONE shared checkout" "$tmp/repo" 1
  AEGIS_TREE_ACTIVITY_ME=bob   ck "edit reaching that tree through a SYMLINK from outside" "$tmp/symlink-to-repo" 1

  echo "MUST STAY SILENT:"
  AEGIS_TREE_ACTIVITY_ME=bob   ck "agent in its OWN worktree of the same repo" "$tmp/repo-wt-alice" 0
  AEGIS_TREE_ACTIVITY_ME=alice AEGIS_TREE_ACTIVITY_PID=$$ ck "the registering agent itself (no self-warning)" "$tmp/repo" 0
  # a reader never registers, so a tree with no registrations is silent by construction
  AEGIS_TREE_ACTIVITY_ME=carol ck "readers / a coordinator auditing an unregistered tree" "$tmp/repo-wt-alice" 0
  AEGIS_TREE_ACTIVITY_ME=alice AEGIS_TREE_ACTIVITY_PID=$$ "$0" end "$tmp/repo"
  AEGIS_TREE_ACTIVITY_ME=bob   ck "after the mutator finishes" "$tmp/repo" 0
  # stale entries must not haunt the tree
  AEGIS_TREE_ACTIVITY_ME=dave AEGIS_TREE_ACTIVITY_PID=$$ "$0" begin "$tmp/repo" "git pull"
  AEGIS_TREE_ACTIVITY_STALE=-1 AEGIS_TREE_ACTIVITY_ME=bob ck "a STALE entry (dead/aged-out session)" "$tmp/repo" 0

  rm -rf "$tmp"
  echo; echo "  $pass passed, $fail failed"
  [ "$fail" = 0 ] || return 1
}

case "${1:---selftest}" in
  begin) shift; cmd_begin "$@" ;;
  end)   shift; cmd_end   "$@" ;;
  check) shift; cmd_check "$@" ;;
  list)  cmd_list ;;
  --selftest) selftest ;;
  *) echo "usage: $0 {begin|end|check <path>|list|--selftest}" >&2 ;;
esac
exit 0
