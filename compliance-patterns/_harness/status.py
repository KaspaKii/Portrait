#!/usr/bin/env python3
"""
_harness/status.py — snapshot of project phase progress, test counts, patterns,
primitives, and KIPs in flight for `kaspa-compliance-patterns`.

Honest-by-design: reads what's actually in the repo + cargo, never asserts
unverified state. Cheap to run; intended for daily / per-session use.

Usage:
    python3 _harness/status.py            # text snapshot
    python3 _harness/status.py --json     # machine-readable
"""
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES_DIR = ROOT / "crates"
EXAMPLES_DIR = ROOT / "examples"
BOOK_DIR = ROOT / "book"
CHANGELOG = ROOT / "CHANGELOG.md"


def sh(cmd, cwd=ROOT, check=False):
    """Run a shell command and return (rc, stdout). Captured. Never raises by default."""
    p = subprocess.run(cmd, cwd=str(cwd), shell=isinstance(cmd, str),
                       capture_output=True, text=True, check=check)
    return p.returncode, p.stdout.strip(), p.stderr.strip()


def git_head_short():
    rc, out, _ = sh(["git", "rev-parse", "--short", "HEAD"])
    return out if rc == 0 else "?"


def git_branch():
    rc, out, _ = sh(["git", "branch", "--show-current"])
    return out if rc == 0 else "?"


def git_dirty():
    rc, out, _ = sh(["git", "status", "--porcelain"])
    return bool(out.strip())


def list_crates():
    if not CRATES_DIR.is_dir():
        return []
    return sorted(d.name for d in CRATES_DIR.iterdir()
                  if d.is_dir() and (d / "Cargo.toml").exists())


def list_examples():
    if not EXAMPLES_DIR.is_dir():
        return []
    return sorted(d.name for d in EXAMPLES_DIR.iterdir()
                  if d.is_dir() and (d / "Cargo.toml").exists())


def cargo_test_count():
    """Run cargo test --workspace and return (passed, failed, ignored)."""
    rc, out, err = sh(["cargo", "test", "--workspace", "--no-fail-fast", "-q"])
    full = out + "\n" + err
    p, f, i = 0, 0, 0
    for m in re.finditer(r"(\d+)\s+passed", full):
        p += int(m.group(1))
    for m in re.finditer(r"(\d+)\s+failed", full):
        f += int(m.group(1))
    for m in re.finditer(r"(\d+)\s+ignored", full):
        i += int(m.group(1))
    return p, f, i, rc


def cargo_fmt_clean():
    rc, _, _ = sh(["cargo", "fmt", "--check"])
    return rc == 0


def cargo_clippy_clean():
    rc, _, _ = sh(["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
    return rc == 0


def primitives_extracted():
    """Count public modules in kcp-common::* (the EVM-pattern-equivalent
    primitives layer per the Phase 2 plan)."""
    lib = CRATES_DIR / "kcp-common" / "src" / "lib.rs"
    if not lib.exists():
        return 0, []
    mods = re.findall(r"^pub mod (\w+)", lib.read_text(), flags=re.M)
    return len(mods), mods


def changelog_current_version():
    if not CHANGELOG.exists():
        return None
    text = CHANGELOG.read_text()
    m = re.search(r"^## \[(\d+\.\d+\.\d+)\]", text, flags=re.M)
    return m.group(1) if m else None


def main():
    as_json = "--json" in sys.argv

    crates = list_crates()
    examples = list_examples()
    primitive_count, primitives = primitives_extracted()
    cl_version = changelog_current_version()

    # Cargo gates — run them so the snapshot reflects reality.
    p, f, i, test_rc = cargo_test_count()
    fmt_ok = cargo_fmt_clean()
    clippy_ok = cargo_clippy_clean()

    data = {
        "project": "kaspa-compliance-patterns",
        "steward": "Stichting Kii Foundation",
        "engine_ref": "rusty-kaspa v2.0.0 (90dbf07)",
        "git": {
            "branch": git_branch(),
            "head": git_head_short(),
            "dirty": git_dirty(),
            "remote_configured": bool(sh(["git", "remote"])[1]),
        },
        "crates": crates,
        "crates_count": len(crates),
        "examples": examples,
        "primitives_in_kcp_common": primitives,
        "primitives_count": primitive_count,
        "tests": {
            "passed": p,
            "failed": f,
            "ignored": i,
            "exit_code": test_rc,
        },
        "gates": {
            "fmt": fmt_ok,
            "clippy_workspace_all_targets_D_warnings": clippy_ok,
            "test_workspace": test_rc == 0,
        },
        "changelog_current_version": cl_version,
        "book_present": (BOOK_DIR / "book.toml").exists(),
    }

    if as_json:
        print(json.dumps(data, indent=2))
        return

    # Human snapshot
    print("kaspa-compliance-patterns — status snapshot")
    print("=" * 50)
    print(f"steward:  {data['steward']}")
    print(f"engine:   {data['engine_ref']}")
    print(f"branch:   {data['git']['branch']} @ {data['git']['head']}"
          f" ({'DIRTY' if data['git']['dirty'] else 'clean'})")
    print(f"remote:   {'configured' if data['git']['remote_configured'] else 'NONE (pre-Foundation-org)'}")
    print()
    print(f"crates ({len(crates)}):       {', '.join(crates)}")
    print(f"examples:           {', '.join(examples) if examples else '(none)'}")
    print(f"primitives in kcp-common ({primitive_count}): {', '.join(primitives)}")
    print(f"changelog version:  {cl_version or '(no version yet)'}")
    print(f"book.toml:          {'present' if data['book_present'] else 'MISSING'}")
    print()
    print("GATES")
    print(f"  cargo fmt --check        : {'PASS' if fmt_ok else 'FAIL'}")
    print(f"  cargo clippy -D warnings : {'PASS' if clippy_ok else 'FAIL'}")
    print(f"  cargo test --workspace   : {p} passed / {f} failed / {i} ignored"
          f" (exit {test_rc})")
    print()
    overall = all([fmt_ok, clippy_ok, test_rc == 0, f == 0])
    print(f"OVERALL: {'GREEN' if overall else 'RED'}")


if __name__ == "__main__":
    main()
