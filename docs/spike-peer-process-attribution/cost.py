import os, glob, time, statistics

def scan(uid_filter=None):
    t0 = time.perf_counter()
    n = 0
    for pdir in glob.glob("/proc/[0-9]*"):
        if uid_filter is not None:
            try:
                if os.stat(pdir).st_uid != uid_filter:
                    continue
            except OSError:
                continue
        try:
            for fd in os.listdir(f"{pdir}/fd"):
                n += 1
                try:
                    os.readlink(f"{pdir}/fd/{fd}")
                except OSError:
                    pass
        except OSError:
            continue
    return (time.perf_counter() - t0) * 1000, n

for label, uf in (("all processes", None), (f"uid={os.getuid()} only", os.getuid())):
    runs = [scan(uf) for _ in range(5)]
    ms = [r[0] for r in runs]
    print(f"{label:22} median {statistics.median(ms):6.1f}ms   fds {runs[0][1]}")
print(f"\nprocs on this machine: {len(glob.glob('/proc/[0-9]*'))}")
