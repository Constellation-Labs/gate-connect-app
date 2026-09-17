//! Does launchd actually do what the restart policy assumes?
//!
//! [`gate_connect_core::crash_restart`] rests on three claims about a
//! `KeepAlive` dictionary naming only `Crashed`, and all three are load-bearing:
//!
//! 1. A process that dies from a crash signal is started again.
//! 2. A process that exits cleanly is left alone, or quitting the app would
//!    resurrect it.
//! 3. A process killed with `SIGKILL` is left alone, because that is what Force
//!    Quit sends and an app that comes back from a deliberate kill is worse
//!    than one that stays dead.
//!
//! Those are properties of launchd, not of our code, so nothing in a unit test
//! can establish them: the unit tests prove we write the keys we meant to write,
//! and this proves the keys mean what we think. It runs the real `launchctl`
//! against a plist produced by the real [`crash_restart::arm`], in the shape the
//! autostart plugin writes, so the artifact under test is the one that ships.
//!
//! **Ignored by default, on purpose.** CI runs `cargo test --workspace` on
//! macOS, and this test needs a GUI session `launchctl` can bootstrap into,
//! which a runner may not have, and it waits out a throttle interval. Run it
//! deliberately on a real machine:
//!
//! ```text
//! cargo test -p gate-connect-core --test launchd_restart -- --ignored --nocapture
//! ```
//!
//! Promote it to a default test once it has passed somewhere that CI can also
//! reach. It takes a little over [`crash_restart::THROTTLE_SECONDS`] to run:
//! the three jobs are bootstrapped together and waited on once, rather than
//! each paying its own interval.
//!
//! It touches nothing of the user's. Every job has a label unique to this
//! process, the plists live in a scratch directory rather than
//! `~/Library/LaunchAgents`, and each job is booted out again on the way past,
//! including when an assertion fails.
#![cfg(target_os = "macos")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use gate_connect_core::crash_restart;

/// How the test program ends, which is the whole variable under test.
#[derive(Clone, Copy)]
enum Ending {
    /// `SIGILL`, one of the signals launchd counts as a crash.
    Crash,
    /// Exit status zero.
    Clean,
    /// `SIGKILL`, what Force Quit sends.
    Killed,
}

impl Ending {
    fn label(self) -> &'static str {
        match self {
            Ending::Crash => "crash",
            Ending::Clean => "clean",
            Ending::Killed => "killed",
        }
    }

    /// The line that ends the script. `$$` is the script's own pid, so it
    /// signals itself and launchd sees the job die that way.
    fn script_ending(self) -> &'static str {
        match self {
            Ending::Crash => "kill -ILL $$",
            Ending::Clean => "exit 0",
            Ending::Killed => "kill -KILL $$",
        }
    }

    /// Runs we expect to see after the wait: two means launchd started it
    /// again, one means it let the job lie.
    fn expected_runs(self) -> usize {
        match self {
            Ending::Crash => 2,
            Ending::Clean | Ending::Killed => 1,
        }
    }
}

/// One bootstrapped launchd job, booted out when it goes out of scope so a
/// failed assertion cannot leave a job loaded in the session.
struct Job {
    label: String,
    uid: String,
    tally: PathBuf,
}

impl Job {
    /// Write the script and a plist in the shape the autostart plugin writes,
    /// arm it with the real production code, and hand it to launchd.
    fn start(dir: &Path, uid: &str, ending: Ending) -> Job {
        let label = format!(
            "ai.constellation.gate-connect.launchd-test.{}.{}",
            std::process::id(),
            ending.label()
        );
        let tally = dir.join(format!("{}.runs", ending.label()));
        let script = dir.join(format!("{}.sh", ending.label()));

        // One line appended per run, so the file's length is the run count.
        fs::write(
            &script,
            format!(
                "#!/bin/sh\necho run >> {tally}\n{end}\n",
                tally = tally.display(),
                end = ending.script_ending(),
            ),
        )
        .expect("writing the test script");
        Command::new("/bin/chmod")
            .arg("+x")
            .arg(&script)
            .status()
            .expect("chmod");

        let plist = dir.join(format!("{label}.plist"));
        fs::write(&plist, plugin_shaped_plist(&label, &script)).expect("writing the plist");

        // The code under test. Without this the job has no KeepAlive at all and
        // every ending below would look like "launchd left it alone", so a
        // silent failure here would read as three passes.
        assert!(
            crash_restart::arm(&plist).expect("arming the plist"),
            "arm() reported no change, so the policy under test was never written"
        );
        let armed = fs::read_to_string(&plist).expect("reading back the plist");
        assert!(
            armed.contains("<key>Crashed</key>"),
            "armed plist is missing the key this test exists to exercise:\n{armed}"
        );

        let out = Command::new("/bin/launchctl")
            .arg("bootstrap")
            .arg(format!("gui/{uid}"))
            .arg(&plist)
            .output()
            .expect("running launchctl bootstrap");
        assert!(
            out.status.success(),
            "launchctl bootstrap failed for {label}: {}\n\
             A GUI session is needed here; over SSH or on a headless runner \
             there is none to bootstrap into.",
            String::from_utf8_lossy(&out.stderr).trim()
        );

        Job {
            label,
            uid: uid.to_string(),
            tally,
        }
    }

    /// Completed runs so far, counted from the lines the script appended.
    fn runs(&self) -> usize {
        fs::read_to_string(&self.tally)
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        let _ = Command::new("/bin/launchctl")
            .arg("bootout")
            .arg(format!("gui/{}/{}", self.uid, self.label))
            .output();
    }
}

/// The template `auto-launch` 0.5 writes, which is what the autostart plugin
/// puts in `~/Library/LaunchAgents`. Reproduced rather than imported because
/// the point is to arm the same shape the real file has.
fn plugin_shaped_plist(label: &str, program: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n  \
         <dict>\n  \
         <key>Label</key>\n  \
         <string>{label}</string>\n  \
         <key>ProgramArguments</key>\n  \
         <array><string>{program}</string></array>\n  \
         <key>RunAtLoad</key>\n  \
         <true/>\n  \
         </dict>\n\
         </plist>",
        program = program.display(),
    )
}

fn current_uid() -> String {
    let out = Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .expect("running id -u");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
#[ignore = "drives the real launchd; needs a GUI session and waits out a throttle interval"]
fn launchd_restarts_a_crash_and_leaves_a_quit_alone() {
    let dir = std::env::temp_dir().join(format!("gate-launchd-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("creating the scratch directory");
    let uid = current_uid();

    // All three at once, so the wait below is paid once rather than three
    // times. They share nothing but the scratch directory.
    let endings = [Ending::Crash, Ending::Clean, Ending::Killed];
    let jobs: Vec<Job> = endings.iter().map(|&e| Job::start(&dir, &uid, e)).collect();

    // Every job must run once before any claim about restarting means
    // anything: a job that never started would otherwise look like a job
    // launchd declined to restart.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && jobs.iter().any(|j| j.runs() < 1) {
        std::thread::sleep(Duration::from_millis(250));
    }
    for (job, ending) in jobs.iter().zip(endings) {
        assert!(
            job.runs() >= 1,
            "the {} job never ran, so launchd never honoured RunAtLoad",
            ending.label()
        );
    }

    // Now wait out the throttle, with slack: launchd will not respawn a job
    // inside `ThrottleInterval` of its last start, so a restart cannot appear
    // before then, and asserting earlier would fail the crash case for the
    // wrong reason.
    let slack = Duration::from_secs(20);
    let settle = Duration::from_secs(u64::from(crash_restart::THROTTLE_SECONDS)) + slack;
    eprintln!(
        "[launchd-test] waiting {}s for the throttle interval to pass",
        settle.as_secs()
    );
    std::thread::sleep(settle);

    for (job, ending) in jobs.iter().zip(endings) {
        let runs = job.runs();
        match ending {
            Ending::Crash => assert!(
                runs >= ending.expected_runs(),
                "a crashed job was not restarted: {runs} run(s) after {}s. \
                 KeepAlive/Crashed does not mean what crash_restart assumes.",
                settle.as_secs()
            ),
            Ending::Clean => assert_eq!(
                runs,
                ending.expected_runs(),
                "a job that exited cleanly was restarted ({runs} runs). \
                 Quitting the app would resurrect it."
            ),
            Ending::Killed => assert_eq!(
                runs,
                ending.expected_runs(),
                "a SIGKILLed job was restarted ({runs} runs). \
                 Force Quit would not make the app stay closed."
            ),
        }
    }

    drop(jobs);
    let _ = fs::remove_dir_all(&dir);
}
