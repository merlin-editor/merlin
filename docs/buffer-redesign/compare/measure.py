"""Starts an editor in its own session, then samples the memory (PSS) and CPU
time of every process in that session: measure.py <name> <seconds> <cmd...>"""
import os, subprocess, sys, time, signal

name, secs, cmd = sys.argv[1], float(sys.argv[2]), sys.argv[3:]
tick = os.sysconf("SC_CLK_TCK")
tag = f"MEASURE_TAG={os.getpid()}"
env = dict(os.environ, MEASURE_TAG=str(os.getpid()))
p = subprocess.Popen(cmd, start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)

# Also any process whose command line holds this (a profile folder), for
# programs that start themselves again with a cleaned environment.
mark = os.environ.get("MEASURE_MARK", "\0none").encode()
# Or every process running this program, when no other copy runs.
exe = os.environ.get("MEASURE_EXE")

def procs():
    """The run's processes: tagged or marked, and everything under them."""
    info, roots = {}, set()
    for d in os.listdir("/proc"):
        if not d.isdigit():
            continue
        try:
            stat = open(f"/proc/{d}/stat").read()
            fields = stat[stat.rindex(")") + 2:].split()
            pid = int(d)
            info[pid] = (int(fields[1]), int(fields[11]) + int(fields[12]))
            if pid == os.getpid():
                continue
            if exe and os.readlink(f"/proc/{d}/exe") == exe:
                roots.add(pid)
                continue
            if mark in open(f"/proc/{d}/cmdline", "rb").read():
                roots.add(pid)
            elif tag.encode() in open(f"/proc/{d}/environ", "rb").read().split(b"\0"):
                roots.add(pid)
        except (OSError, ValueError):
            pass
    found, grew = set(roots), True
    while grew:
        grew = False
        for pid, (ppid, _) in info.items():
            if pid not in found and ppid in found:
                found.add(pid)
                grew = True
    return [(pid, info[pid][1]) for pid in found if pid in info]

def pss(pid):
    try:
        for line in open(f"/proc/{pid}/smaps_rollup"):
            if line.startswith("Pss:"):
                return int(line.split()[1]) / 1024
    except OSError:
        pass
    return 0.0

start = time.time()
peak = 0.0
samples = []
cpu_seen = {}
while time.time() - start < secs:
    ps = procs()
    for pid, t in ps:
        cpu_seen[pid] = t
    mem = sum(pss(pid) for pid, _ in ps)
    peak = max(peak, mem)
    cpu = sum(cpu_seen.values()) / tick
    samples.append((time.time() - start, mem, cpu, len(ps)))
    time.sleep(0.5)
for sig in (signal.SIGTERM, signal.SIGKILL):
    for pid, _ in procs():
        try:
            os.kill(pid, sig)
        except ProcessLookupError:
            pass
    time.sleep(1.5)
t, mem, cpu, n = samples[-1]
# CPU over the last 10 seconds, when it should be idle.
idle = [s for s in samples if s[0] >= t - 10]
idle_cpu = (idle[-1][2] - idle[0][2]) / max(idle[-1][0] - idle[0][0], 0.1)
print(f"{name}: settled {mem:7.0f} MB, peak {peak:7.0f} MB, CPU {cpu:6.1f} s in {t:.0f} s, "
      f"idle {idle_cpu * 100:5.1f}% of a core, {n} processes")
