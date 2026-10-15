# Testing

One command runs the pull-request test tiers locally against the mock
gateway, with no credentials:

```
bash scripts/test-tiers.sh                    # all four tiers
bash scripts/test-tiers.sh --tier daemon      # one tier
bash scripts/test-tiers.sh --tier workspace,routing
```

| Flag | Meaning |
| --- | --- |
| `--tier NAME` | Run only this tier: `workspace`, `daemon`, `app`, `routing`. Repeatable, or comma separated. |
| `--bundle DIR` | A directory holding the Linux `.deb` for the `app` tier. |
| `--from-run ID` | Download the `.deb` from that CI run's `bundle-ubuntu-22.04` artifact (needs `gh auth login`). |
| `--fail-fast` | Stop at the first failed tier. By default every selected tier runs. |
| `--list` | Print the tier names. |

It needs Rust (the pinned toolchain), pnpm and Node 20 for the host tiers, and
Docker for the container ones. Every step's output goes to
`target/test-tiers/<timestamp>/<tier>-<step>.log`. A failure prints
`FAIL: tier <tier>, step <step>` with that log's path, and the summary at the
end repeats it with each tier's time. The summary also lists what was not
covered on this host, and why.

No tier needs a Gate key, an OAuth login or a staging tenant. Each one talks to
a loopback mock. The test steps run with the proxy variables unset, as on a CI
runner, so a developer's own running Gate Connect never sees their traffic.

## The tiers

Each tier runs the commands of its CI job in `.github/workflows/ci.yml`, not a
local variant of them.

| Tier | CI job | Where it runs | What it runs |
| --- | --- | --- | --- |
| `workspace` | `rust`, `web` | This host | `scripts/stage-sidecar.sh`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace`; `pnpm install --frozen-lockfile`, `pnpm build`, `pnpm test` (vitest), `pnpm typecheck:e2e`, `pnpm test:e2e` (Playwright, fake backend). |
| `daemon` | `rust` (Linux leg) | Linux host: directly. macOS or Windows: the `daemon` container. | `cargo test -p gate-connect-cli --test helper_daemon`. |
| `app` | `app-e2e-linux` | Always the `app` container, never the host. | Installs the `.deb` and signs in through the app's window with `ci/e2e/app/windows-linux-signin.sh`, under Xvfb, `dbus-run-session` and gnome-keyring. |
| `routing` | `catalog-routing` (default mode) | This host | `cargo test -p gate-connect-core --test catalog_routing`, with a pinned throwaway CA. |

## Which tier catches what

| Class of bug | Caught by |
| --- | --- |
| Logic in the core, CLI or desktop crates; config writers; keychain seams (in-memory and file backends) | `workspace` (cargo test) |
| Lints, formatting, a cfg-gated module that no longer compiles on this OS | `workspace` (clippy, rustfmt) |
| Frontend logic, components, types | `workspace` (vitest, `pnpm build`) |
| The frontend against a fake Tauri backend: flows, copy, navigation | `workspace` (`pnpm test:e2e`) |
| The Linux helper daemon across a process boundary: a probe that kills the live daemon, clients served one at a time, a lost port, signal handling | `daemon` |
| The shipped release bundle: install, first launch, intro, API key sign-in, the key landing in the OS secret store and in no file, the request reaching the gateway with the right headers | `app` |
| A catalog entry that no longer routes: a missing host or prefix, the wrong `x-gate-client`, missing Gate headers | `routing` |

## What stays in CI

Some checks write machine-wide state, so their scripts refuse to run anywhere
but a GitHub-hosted runner. The command says each time that it skipped them.

- **Catalog routing, OS-trust mode.** `ci/catalog-routing/run.sh` trusts the CA
  in the machine store and refuses outside CI. Locally the suite runs in its
  default mode, which pins the CA and never reads the OS store.
- **UI e2e against the live backend, routing arc** (`ui-e2e`,
  `GATE_UI_HARNESS_ROUTING=1`). It installs a root CA with `sudo -n`.
- **The macOS and Windows installed-app suites** (`app-e2e-macos`,
  `app-e2e-windows`). See below.

## The installed-app tier

The installed-app suite edits the hosts file, trusts a root machine-wide, binds
port 443 and replaces the installed app. None of that belongs on a developer's
machine, so it never runs on the host. Its scripts refuse to run anywhere
except a GitHub-hosted runner, or the test container described here.

### Linux, in a container

`docker/test-tiers/Dockerfile` (target `app`) is Ubuntu 22.04, like the CI
runner. It carries the `app-e2e-linux` job's packages, Node 20, tauri-driver
and every runtime dependency of the `.deb`. The command starts it with:

- `--network none`, so nothing inside can reach the network. The mock gateway
  listens on the container's own loopback;
- the checkout mounted read-only at `/src`, and the bundle read-only at
  `/bundle`;
- a writable `/out`, which ends up in
  `target/test-tiers/<timestamp>/app-e2e-out/` with the screenshots, page dumps
  and logs the CI job uploads.

The hosts file, the trust store, port 443 and the install all belong to the
container, and go away with it (`--rm`).

`windows-linux-signin.sh` accepts the container only when it can verify it is
inside one: the image sets `GATE_CONNECT_E2E_CONTAINER=1`, the container
runtime creates `/.dockerenv`, and PID 1 is not an init system. All three must
hold, so setting the variable on a host does nothing. The GitHub-hosted check
is unchanged.

The `.deb` is amd64. On an Apple Silicon Mac or an arm64 Linux host the
container runs under emulation, which is slow but needs nothing else. With no
`--bundle` or `--from-run`, the command takes the bundle from the newest green
CI run of the current branch. It warns when that run built a different commit
than `HEAD`.

### macOS and Windows

The macOS driver (`ci/e2e/app/macos-signin.sh`, accessibility) and the Windows
driver (`windows-uia.ps1`, UI Automation) need a real macOS or Windows desktop.
No container provides one. On those hosts the `app` tier runs the Linux
container, and the summary says the host's own driver was not covered.

To cover them, use a disposable VM. The simplest one is a GitHub-hosted
runner, which is thrown away after each job. `ci.yml` accepts
`workflow_dispatch`, so this runs every tier, both installed-app jobs
included, on your branch:

```
gh workflow run ci.yml --ref <your-branch>
```

A local disposable VM (a throwaway macOS guest, or Windows Sandbox) is the
other route. The scripts do not accept one yet, because a VM has no marker
they can verify the way they verify the container. Do not remove the guard to
run them in one.

## Container caches

The `daemon` tier keeps the cargo registry and its build output in named
volumes, so a second run does not start cold. `GC_TIERS_VOLUME_PREFIX`
(default `gc-test-tiers-`) names them, and `GC_TIERS_IMAGE` (default
`gate-connect-test-tiers`) names the images. Change both to keep a second
checkout's caches apart. To reclaim the space:

```
docker volume rm gc-test-tiers-cargo-registry gc-test-tiers-cargo-git gc-test-tiers-daemon-target
```

Crates are fetched with the network on. The build and the tests then run with
`--network none`.
