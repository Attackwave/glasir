#!/usr/bin/env python3
"""Scaling gate: the same work on a 100k-line and a 1M-line tree, as a ratio.

Every scaling wall this project found was invisible on its own tree and on any
tree small enough for a self-check. An absolute bound measures the machine — a
CI runner is two to five times slower than a workstation, and noisier — so this
compares a tree against one ten times smaller on the same machine in the same
run. Linear work reads about 10x; a quadratic wall reads 100x.

    python3 bench/scale.py target/release/glasir            # report
    python3 bench/scale.py target/release/glasir --check    # fail over a bound

Bounds live in bench/scale.txt. Linux only: peak memory comes from wait4.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
SMALL, BIG = 100_000, 1_000_000
# Below this a timing is noise, not a measurement: a 0.2 ms call against a
# 3 ms one is not a 15x regression.
FLOOR_MS = 20.0


def make(root, lines):
    subprocess.run([sys.executable, os.path.join(HERE, "make_tree.py"), root, str(lines)],
                   check=True, stdout=subprocess.DEVNULL)
    # Documents, which make_tree.py does not write: one per directory, naming
    # functions in backticks. The Markdown wall was invisible without them.
    src = os.path.join(root, "src")
    os.makedirs(os.path.join(root, "docs"), exist_ok=True)
    for d in sorted(os.listdir(src)):
        n = int(d[3:])
        names = " ".join(f"`op_{n}_{f}_0`" for f in range(0, 50, 5))
        with open(os.path.join(root, "docs", f"{d}.md"), "w") as out:
            out.write(f"# {d}\n\n## Paths\n\nThe {d} paths go through {names}.\n")
    subprocess.run(["git", "init", "-q", root], check=True)
    subprocess.run(["git", "-C", root, "add", "-A"], check=True)
    subprocess.run(["git", "-C", root, "-c", "user.name=s", "-c", "user.email=s@s",
                    "-c", "commit.gpgsign=false", "commit", "-qm", "tree"], check=True)


def run(cmd):
    """Wall time in seconds and peak RSS in MB of one child."""
    t = time.perf_counter()
    p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    _, status, usage = os.wait4(p.pid, 0)
    if status != 0:
        raise SystemExit(f"{cmd} exited with {status}")
    return time.perf_counter() - t, usage.ru_maxrss / 1024


def tools(binary, root):
    """Milliseconds per tool, first call — the one that builds any cache."""
    f = "src/mod000/f00.rs"
    calls = [
        ("overview", {}),
        ("query_graph", {"query": "handles case path"}),
        ("shortest_path", {"from": f"{f}#op_0_0_1", "to": f"{f}#op_0_0_2"}),
        ("explain_node", {"symbol": f"{f}#op_0_0_1"}),
        ("impact", {"symbol": f"{f}#op_0_0_1"}),
        ("cycles", {}),
        ("get_code_snippet", {"symbol": f"{f}#op_0_0_1"}),
        ("find_callers", {"symbol": f"{f}#op_0_0_1"}),
        ("detect_changes", {}),
        ("affected_tests", {"symbol": f"{f}#op_0_0_1"}),
        ("co_changes", {"file": f}),
        ("check_architecture", {"rules": "no-cycles"}),
        ("find_unused", {}),
    ]
    p = subprocess.Popen([binary, "serve", root], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL, text=True)

    def rpc(i, method, params):
        p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params}) + "\n")
        p.stdin.flush()
        return json.loads(p.stdout.readline())

    rpc(0, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                          "clientInfo": {"name": "scale", "version": "0"}})
    out = {}
    for i, (name, args) in enumerate(calls, 1):
        t = time.perf_counter()
        r = rpc(i, "tools/call", {"name": name, "arguments": args})
        out[name] = (time.perf_counter() - t) * 1000
        if r.get("result", {}).get("isError"):
            raise SystemExit(f"{name} refused on the generated tree: {r['result']['content'][0]['text']}")
    with open(f"/proc/{p.pid}/status") as s:
        rss = next(int(l.split()[1]) for l in s if l.startswith("VmHWM")) / 1024
    p.kill()
    return out, rss


def measure(binary, lines, work):
    root = os.path.join(work, str(lines))
    make(root, lines)
    cold, cold_rss = run([binary, "analyse", root])
    warm, _ = run([binary, "analyse", root])
    per_tool, serve_rss = tools(binary, root)
    shutil.rmtree(root)
    m = {"analyse_cold": cold * 1000, "analyse_warm": warm * 1000,
         "rss_analyse": cold_rss, "rss_serve": serve_rss}
    m.update({f"tool_{k}": v for k, v in per_tool.items()})
    return m


def bounds():
    out = {}
    with open(os.path.join(HERE, "scale.txt")) as f:
        for line in f:
            line = line.split("#")[0].strip()
            if line:
                key, value = line.split()
                out[key] = float(value)
    return out


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    binary = os.path.abspath(sys.argv[1])
    work = tempfile.mkdtemp(prefix="glasir-scale-")
    try:
        small = measure(binary, SMALL, work)
        big = measure(binary, BIG, work)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    limit = bounds()
    failed = []
    print(f"{'':24} {'100k':>10} {'1M':>10} {'ratio':>7} {'bound':>6}")
    for key in small:
        unit = "MB" if key.startswith("rss") else "ms"
        floor = 1.0 if unit == "MB" else FLOOR_MS
        ratio = max(big[key], floor) / max(small[key], floor)
        bound = limit.get(key, limit["default"])
        mark = "" if ratio <= bound else "  REGRESSION"
        if mark:
            failed.append(key)
        print(f"{key:24} {small[key]:>8.1f}{unit} {big[key]:>8.1f}{unit} {ratio:>6.1f}x {bound:>5.0f}x{mark}")
    if "--check" in sys.argv and failed:
        raise SystemExit(f"\nscaling regressed: {', '.join(failed)} grew faster than bench/scale.txt allows")


if __name__ == "__main__":
    main()
