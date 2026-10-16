// Exit when a given process is gone. Opt-in through MOCK_EXIT_WITH_PID; a mock
// started without it lives until it is killed, as before.
//
// Why this exists: on Windows the mocks are started from Git Bash, and a
// Cygwin process that `exec`s a native one exits its own stub, leaving the new
// process with a dead parent PID. `taskkill /T` - what Playwright's webServer
// teardown uses - walks parent PIDs, so it never reaches them, and a mock left
// running holds its port for the next run. `ci/e2e/ui-harness.sh` hands the
// mocks its own Windows PID, which is what taskkill does kill, and they follow
// it down. Unix needs none of this: Playwright kills the whole process group.
export function exitWithPid() {
  const raw = process.env.MOCK_EXIT_WITH_PID;
  if (!raw) return;
  const pid = Number(raw);
  if (!Number.isInteger(pid) || pid <= 0) return;
  const alive = () => {
    try {
      process.kill(pid, 0);
      return true;
    } catch (err) {
      // EPERM is "exists, not ours", which still means alive.
      return err && err.code === 'EPERM';
    }
  };
  setInterval(() => {
    if (!alive()) process.exit(0);
  }, 1000).unref();
}
