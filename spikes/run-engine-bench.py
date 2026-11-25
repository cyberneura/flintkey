#!/usr/bin/env python3
"""Run the two engine spikes in benchmark mode and print a comparison.

Run this from a normal terminal: the apps need a screen and this script needs `ps`.
Each app measures itself and writes JSON. This script adds the memory of the helper
processes that exist only while the app runs (the OS WebView runs in processes separate
from the app itself).

    python3 spikes/run-engine-bench.py            # both engines
    python3 spikes/run-engine-bench.py tauri      # one engine only
"""

import json
import platform
import statistics
import subprocess
import sys
import time
from pathlib import Path

SPIKES = Path(__file__).resolve().parent
RESULTS = SPIKES / "results"
ENGINES = {
    "gpui": SPIKES / "gpui-launcher" / "target" / "release" / "spike-gpui-launcher",
    "tauri": SPIKES / "tauri-launcher" / "target" / "release" / "spike-tauri-launcher",
}
IDLE_TIMEOUT_SECONDS = 180


def process_table():
    """Return pid -> (rss_kb, command) for every process of every user."""
    out = subprocess.run(["ps", "-axo", "pid=,rss=,comm="], capture_output=True, text=True, check=True).stdout
    table = {}
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and parts[0].isdigit():
            table[int(parts[0])] = (int(parts[1]), parts[2])
    return table


def footprint_kb(pid):
    """Read the physical footprint from vmmap. None if the process cannot be inspected."""
    try:
        out = subprocess.run(["vmmap", "--summary", str(pid)], capture_output=True, text=True, timeout=30).stdout
    except (OSError, subprocess.TimeoutExpired):
        return None
    for line in out.splitlines():
        if line.startswith("Physical footprint:"):
            value = line.split(":", 1)[1].strip()
            number, unit = float(value[:-1]), value[-1]
            return int(number * {"K": 1, "M": 1024, "G": 1024 * 1024}[unit])
    return None


def wait_for_phase(path, phase, child):
    deadline = time.time() + IDLE_TIMEOUT_SECONDS
    while time.time() < deadline:
        if path.exists():
            try:
                if json.loads(path.read_text())["phase"] == phase:
                    return True
            except (ValueError, KeyError):
                pass
        if child.poll() is not None:
            return False
        time.sleep(0.2)
    return False


def run(engine):
    binary = ENGINES[engine]
    if not binary.exists():
        sys.exit(f"{binary} is missing. Build it first: (cd {binary.parents[2]} && cargo build --release)")
    RESULTS.mkdir(exist_ok=True)
    out = RESULTS / f"{engine}-{platform.system().lower()}.json"
    out.unlink(missing_ok=True)

    before = process_table()
    child = subprocess.Popen([str(binary), "--bench", str(out)])
    reached_idle = wait_for_phase(out, "idle", child)

    processes = []
    if reached_idle:
        after = process_table()
        # WebKit starts its helpers through launchd, so they are not children of the app.
        # Count the WebKit processes that appeared after launch as belonging to this app.
        for pid, (rss, command) in sorted(after.items()):
            is_app = pid == child.pid
            is_new_helper = pid not in before and "WebKit" in command
            if is_app or is_new_helper:
                processes.append(
                    {"pid": pid, "command": command.rsplit("/", 1)[-1], "rss_kb": rss, "footprint_kb": footprint_kb(pid)}
                )
    try:
        child.wait(timeout=60)
    except subprocess.TimeoutExpired:
        # An app left running keeps its window on screen and blocks the next engine's run.
        child.kill()
        child.wait()

    report = json.loads(out.read_text()) if out.exists() else {"engine": engine, "notes": ["no report written"]}
    report["processes_while_idle"] = processes
    report["exit_code"] = child.returncode
    report["host"] = {"system": platform.system(), "release": platform.release(), "machine": platform.machine()}
    out.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    return report


def summarize(values):
    present = [v for v in values if v is not None]
    if not present:
        return "no data"
    missing = len(values) - len(present)
    ordered = sorted(present)
    p90 = ordered[min(len(ordered) - 1, round(0.9 * (len(ordered) - 1)))]
    text = f"median {statistics.median(present):6.1f}  p90 {p90:6.1f}  max {ordered[-1]:6.1f}  (n={len(present)})"
    return text + (f"  MISSING {missing}" if missing else "")


def print_report(report):
    print(f"\n=== {report.get('engine')} ===")
    cold = report.get("cold_start_ms")
    print(f"cold start -> first frame   : {cold:.1f} ms" if cold is not None else "cold start -> first frame   : no data")
    print(f"show -> first frame (ms)    : {summarize(report.get('show_ms', []))}")
    if report.get("show_visible_ms"):
        print(f"show -> OS says visible (ms): {summarize(report['show_visible_ms'])}")
    if report.get("reopen_ms"):
        print(f"reopen window (ms)          : {summarize(report['reopen_ms'])}")
    print(f"key -> frame (ms)           : {summarize(report.get('type_total_ms', []))}")
    print(f"  of which matcher (us)     : {summarize(report.get('type_filter_us', []))}")
    print(f"typed text arrived          : {report.get('typed_query')!r}")
    total_rss = 0
    for process in report.get("processes_while_idle", []):
        total_rss += process["rss_kb"]
        footprint = process["footprint_kb"]
        footprint_text = f"{footprint / 1024:7.1f} MB" if footprint is not None else "      n/a"
        print(f"  {process['command'][:44]:44} rss {process['rss_kb'] / 1024:7.1f} MB  footprint {footprint_text}")
    print(f"memory while idle (rss sum) : {total_rss / 1024:.1f} MB")
    for note in report.get("notes", []):
        print(f"note: {note}")
    if report.get("exit_code") not in (0, None):
        print(f"note: exited with code {report['exit_code']}")


def main():
    engines = sys.argv[1:] or list(ENGINES)
    unknown = [e for e in engines if e not in ENGINES]
    if unknown:
        sys.exit(f"unknown engine {unknown}; choose from {list(ENGINES)}")
    print("Do not touch the keyboard or mouse while the windows flash (about 40 seconds per engine).")
    reports = [run(engine) for engine in engines]
    for report in reports:
        print_report(report)
    print(f"\nRaw results: {RESULTS}")


if __name__ == "__main__":
    main()
