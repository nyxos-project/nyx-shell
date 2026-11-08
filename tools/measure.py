#!/usr/bin/env python3
"""Idle cost of processes, read from /proc (nothing is started or touched).

    tools/measure.py SECONDS NAME=PID [NAME=PID ...]

For each process: RSS, PSS, private and swapped-out memory (smaps_rollup) at the end, CPU time
and context switches (wakeups) over the interval.
"""
import os
import sys
import time

TICK = os.sysconf("SC_CLK_TCK")


def cpu(pid):
    fields = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / TICK


def switches(pid):
    total = 0
    for line in open(f"/proc/{pid}/status"):
        if line.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")):
            total += int(line.split()[1])
    return total


def memory(pid):
    out = {}
    for line in open(f"/proc/{pid}/smaps_rollup"):
        parts = line.split()
        if parts[0] in ("Rss:", "Pss:", "Private_Clean:", "Private_Dirty:", "Swap:"):
            out[parts[0][:-1]] = int(parts[1]) / 1024
    return out["Rss"], out["Pss"], out["Private_Clean"] + out["Private_Dirty"], out.get("Swap", 0)


def main():
    seconds = float(sys.argv[1])
    procs = [a.split("=", 1) for a in sys.argv[2:]]
    start = {name: (cpu(pid), switches(pid)) for name, pid in procs}
    time.sleep(seconds)
    print(f"| Process | RSS | PSS | Private | Swapped | CPU over {seconds:.0f} s | Wakeups/s |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for name, pid in procs:
        try:
            c0, s0 = start[name]
            rss, pss, private, swap = memory(pid)
            used = cpu(pid) - c0
            wake = (switches(pid) - s0) / seconds
            print(f"| {name} | {rss:.1f} MiB | {pss:.1f} MiB | {private:.1f} MiB | {swap:.1f} MiB | {used:.2f} s ({used / seconds * 100:.2f} %) | {wake:.1f} |")
        except FileNotFoundError:
            print(f"| {name} | gone | | | | | |")


if __name__ == "__main__":
    main()
