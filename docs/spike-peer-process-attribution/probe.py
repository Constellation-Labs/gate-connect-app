"""Peer-process attribution probe.

Answers one question: when a client connects to a loopback listener, what can
the listener learn about which program it is? Mirrors what the Gate engine
would do on accept.

Chain on Linux: peer addr -> /proc/net/tcp row -> socket inode -> scan
/proc/<pid>/fd for socket:[inode] -> /proc/<pid>/{comm,exe,cmdline}.
"""
import os, socket, subprocess, sys, threading, time, glob

def inode_for_peer(ip: str, port: int):
    want_ip = "".join(f"{int(o):02X}" for o in reversed(ip.split(".")))
    want = f"{want_ip}:{port:04X}"
    with open("/proc/net/tcp") as f:
        for line in f.readlines()[1:]:
            fields = line.split()
            if fields[1] == want:          # local_address of the CLIENT socket
                return fields[9], fields[7]  # inode, uid
    return None, None

def pid_for_inode(inode: str):
    target = f"socket:[{inode}]"
    scanned = 0
    for fd in glob.glob("/proc/[0-9]*/fd/*"):
        scanned += 1
        try:
            if os.readlink(fd) == target:
                return fd.split("/")[2], scanned
        except OSError:
            continue
    return None, scanned

def describe(pid: str):
    out = {}
    for name, path in (("comm", "comm"), ("cmdline", "cmdline")):
        try:
            with open(f"/proc/{pid}/{path}", "rb") as f:
                out[name] = f.read().replace(b"\0", b" ").decode(errors="replace").strip()
        except OSError as e:
            out[name] = f"<{e.strerror}>"
    try:
        out["exe"] = os.readlink(f"/proc/{pid}/exe")
    except OSError as e:
        out["exe"] = f"<{e.strerror}>"
    return out

def serve(sock, results):
    conn, peer = sock.accept()
    t0 = time.perf_counter()
    inode, uid = inode_for_peer(peer[0], peer[1])
    t_inode = time.perf_counter()
    pid, scanned = pid_for_inode(inode) if inode else (None, 0)
    t_pid = time.perf_counter()
    info = describe(pid) if pid else {}
    t_end = time.perf_counter()
    results.update(peer=peer, uid=uid, inode=inode, pid=pid, fds_scanned=scanned,
                   ms_inode=(t_inode-t0)*1000, ms_pid=(t_pid-t_inode)*1000,
                   ms_describe=(t_end-t_pid)*1000, **info)
    conn.close()

def run(label, argv):
    sock = socket.socket(); sock.bind(("127.0.0.1", 0)); sock.listen(1)
    port = sock.getsockname()[1]
    results = {}
    t = threading.Thread(target=serve, args=(sock, results)); t.start()
    subprocess.run(argv + [str(port)], capture_output=True, timeout=20)
    t.join(timeout=20)
    sock.close()
    print(f"\n=== {label} ===")
    for k in ("pid", "uid", "comm", "exe", "cmdline", "fds_scanned"):
        print(f"  {k:12} {results.get(k)}")
    print(f"  {'timing':12} inode {results.get('ms_inode',0):.1f}ms  "
          f"pid-scan {results.get('ms_pid',0):.1f}ms  describe {results.get('ms_describe',0):.1f}ms")

CONNECT = "import socket,sys; s=socket.create_connection(('127.0.0.1',int(sys.argv[1]))); import time; time.sleep(0.5)"
if __name__ == "__main__":
    run("python3 (stands in for Hermes)", [sys.executable, "-c", CONNECT])
    run("curl", ["sh", "-c", 'exec curl -s --max-time 2 http://127.0.0.1:"$0"', ])

# Re-run for the console-script shape a pip/uv install actually produces.
if os.environ.get("SHIM"):
    run("venv console script (real Hermes shape)", [os.environ["SHIM"]])

if os.environ.get("MODRUN"):
    run("python3 -m hermes_cli", [sys.executable, "-m", "hermes_cli"])
if os.environ.get("LONG"):
    run("long console-script name (comm is 15 chars)", [os.environ["LONG"]])
