#!/usr/bin/env python3
"""Guard A (aegis-sem1z): flag a RE-READ that was genuinely redundant.

Stiwi: "you already read that file, no need to read it again if it wasn't updated."

The whole feature is the DISCRIMINATION, not the detection. Detecting "this path
was read twice" is trivial and useless — most repeat reads are correct, and a
guard that fires on them is worse than absent because it gets ignored.

FIRES on:
  - re-read of an OVERLAPPING REGION of the same file, with no intervening edit
    and no intervening compaction. The content is still in context; reading it
    again buys nothing.

STAYS SILENT on:
  1. re-read after an edit to that file        -> content genuinely changed
  2. re-read after a COMPACTION boundary       -> the content was dropped from
                                                  context and the agent cannot
                                                  know it is repeating itself
  3. re-read of a DISJOINT REGION              -> a different part of the file
                                                  was never read at all
  4. the first read of anything

Case 3 is NOT in the bead. It came out of live transcript data: this session read
scripts/deploy-cutover.sh twice, at offset=44 limit=60 and offset=30 limit=14 —
disjoint line ranges, both necessary. A guard keyed on file_path alone reports
that as waste, i.e. it fires on correct behaviour, which is exactly the failure
mode the bead warns about. Reading is REGION-scoped, so identity must be too.

Reads the Claude Code session transcript (JSONL). Advisory only: exit 0 always.

Usage:
  session-reread-guard.py <transcript.jsonl>   # report redundant re-reads
  session-reread-guard.py --selftest           # the acceptance, as a test
"""
import json
import sys

WHOLE_FILE = (0, float("inf"))


def region(inp):
    """Line range a Read covered. No offset/limit means the whole file."""
    off, lim = inp.get("offset"), inp.get("limit")
    if off is None and lim is None:
        return WHOLE_FILE
    start = off or 0
    return (start, start + lim) if lim else (start, float("inf"))


def overlaps(a, b):
    return a[0] < b[1] and b[0] < a[1]


def is_compaction(rec):
    """A boundary after which prior content may no longer be in context.

    Several markers are accepted because the field has moved between Claude Code
    versions; treating an unknown-but-compaction-shaped record as a boundary is
    the SAFE direction — it suppresses a warning rather than inventing one.
    """
    if rec.get("isCompactSummary") or rec.get("compact"):
        return True
    if rec.get("subtype") in ("compact_boundary", "compaction", "compact"):
        return True
    msg = rec.get("message") or {}
    return bool(msg.get("isCompactSummary"))


def events(path):
    """Flatten the transcript into an ordered event stream."""
    out = []
    with open(path, errors="ignore") as fh:
        for line in fh:
            try:
                rec = json.loads(line)
            except Exception:
                continue
            if is_compaction(rec):
                out.append({"kind": "compact"})
                continue
            if rec.get("type") != "assistant":
                continue
            for c in (rec.get("message", {}) or {}).get("content") or []:
                if not isinstance(c, dict) or c.get("type") != "tool_use":
                    continue
                inp = c.get("input") or {}
                # A Bash command that so much as NAMES a file is treated as a
                # possible edit to it. Coarse on purpose — measured on 433 real
                # transcripts, Bash-mediated edits (sed -i, a python patch, a
                # heredoc redirect) accounted for an upper bound of 89% of
                # findings, because the guard could not see them and called the
                # honest re-read that followed "waste". Erring toward SILENCE is
                # the only safe direction: a missed redundant read costs tokens,
                # a false accusation costs the guard its credibility.
                if c.get("name") == "Bash":
                    out.append({"kind": "bash", "cmd": str(inp.get("command", ""))})
                    continue
                fp = inp.get("file_path")
                if not fp:
                    continue
                if c.get("name") == "Read":
                    out.append({"kind": "read", "path": fp, "region": region(inp)})
                elif c.get("name") in ("Edit", "Write", "NotebookEdit"):
                    out.append({"kind": "edit", "path": fp})
    return out


def analyse(evs):
    """Return the redundant re-reads. seen[path] = list of live regions."""
    seen, findings = {}, []
    for i, e in enumerate(evs):
        if e["kind"] == "compact":
            seen.clear()          # everything before is no longer reliably in context
        elif e["kind"] == "edit":
            seen.pop(e["path"], None)   # content changed; a re-read is now correct
        elif e["kind"] == "bash":
            cmd = e["cmd"]
            for path in list(seen):
                base = path.rsplit("/", 1)[-1]
                if base and (base in cmd or path in cmd):
                    seen.pop(path, None)    # may have been written outside the Edit tool
        elif e["kind"] == "read":
            prior = seen.get(e["path"], [])
            if any(overlaps(e["region"], p) for p in prior):
                findings.append({"index": i, "path": e["path"], "region": e["region"]})
            seen.setdefault(e["path"], []).append(e["region"])
    return findings


def report(path):
    f = analyse(events(path))
    if not f:
        return
    print("⚠ REDUNDANT RE-READ — this content should still be in your context")
    for x in f:
        lo, hi = x["region"]
        span = "whole file" if (lo, hi) == WHOLE_FILE else f"lines {lo}-{hi}"
        print(f"    {x['path']} ({span})")
    print("  Advisory. Silent on re-reads after an edit, after a compaction, and")
    print("  on a different region of the same file — those are all legitimate.")


# ── acceptance: the bead's discrimination test, executable ──────────────────
def selftest():
    def R(p, off=None, lim=None):
        i = {"file_path": p}
        if off is not None:
            i["offset"] = off
        if lim is not None:
            i["limit"] = lim
        return {"type": "assistant",
                "message": {"content": [{"type": "tool_use", "name": "Read", "input": i}]}}

    def E(p):
        return {"type": "assistant",
                "message": {"content": [{"type": "tool_use", "name": "Edit",
                                         "input": {"file_path": p, "old_string": "a",
                                                   "new_string": "b"}}]}}

    def B(cmd):
        return {"type": "assistant",
                "message": {"content": [{"type": "tool_use", "name": "Bash",
                                         "input": {"command": cmd}}]}}

    C = {"isCompactSummary": True}

    cases = [
        ("MUST FIRE   wasteful re-read, same whole file, nothing between",
         [R("/x.rs"), R("/x.rs")], 1),
        ("MUST FIRE   wasteful re-read, OVERLAPPING regions",
         [R("/x.rs", 10, 50), R("/x.rs", 30, 50)], 1),
        ("must stay SILENT  re-read after an EDIT",
         [R("/x.rs"), E("/x.rs"), R("/x.rs")], 0),
        ("must stay SILENT  re-read after COMPACTION",
         [R("/x.rs"), C, R("/x.rs")], 0),
        ("must stay SILENT  re-read of a DISJOINT region (live-data case)",
         [R("/x.rs", 44, 60), R("/x.rs", 30, 14)], 0),
        ("must stay SILENT  first read of a file",
         [R("/x.rs")], 0),
        ("must stay SILENT  different files",
         [R("/x.rs"), R("/y.rs")], 0),
        ("must stay SILENT  edit to ANOTHER file does not license a re-read",
         [R("/x.rs"), E("/y.rs"), R("/x.rs")], 1),
        ("must stay SILENT  file written via BASH between the reads (89% of real findings)",
         [R("/x.rs"), B("sed -i s/a/b/ /x.rs"), R("/x.rs")], 0),
        ("MUST FIRE   an UNRELATED bash command does not license a re-read",
         [R("/x.rs"), B("ls -la /tmp"), R("/x.rs")], 1),
    ]

    import tempfile, os
    npass = nfail = 0
    for name, recs, expect in cases:
        fd, tmp = tempfile.mkstemp(suffix=".jsonl")
        with os.fdopen(fd, "w") as fh:
            for r in recs:
                fh.write(json.dumps(r) + "\n")
        got = len(analyse(events(tmp)))
        os.unlink(tmp)
        ok = got == expect
        print(f"  {'PASS' if ok else 'FAIL'}  {name}" + ("" if ok else f"  (expected {expect}, got {got})"))
        npass, nfail = npass + ok, nfail + (not ok)
    print(f"\n  {npass} passed, {nfail} failed")
    return 0 if nfail == 0 else 1


if __name__ == "__main__":
    arg = sys.argv[1] if len(sys.argv) > 1 else "--selftest"
    if arg == "--selftest":
        selftest()
    else:
        report(arg)
    sys.exit(0)
