# Review: PR #298 against AG-893, AG-887, AG-901

Reviewed 2026-09-17 at head `b4a1238` (2 commits on `fix/ag-893-shell-channel-copy`,
base `feat/new-app-ui` at `b844b95`). CI: 9 of 9 checks green. Local: tsc, e2e
typecheck, 1190 unit, 260 e2e, all pass.

## Verdict

The code does what the commits say and the copy is now true where it was false.
Two gaps remain against AG-893's Expected, one small and fixable in this PR, one a
product default that neither #290 nor #298 touched and that should go back to the
ticket rather than be changed by us.

## AG-893, clause by clause

The ticket's Expected has four clauses. Checked against the merged base plus #298:

| Clause | Met? | Evidence |
| --- | --- | --- |
| Each control states what it covers in terms the user can act on | Yes | Settings row: "Routes every program you start from now on, not only AI tools, and tells Node to trust Gate's certificate." Tray: the frame's copy. Popover blurb: the same sentence plus the inspect-only sentence. |
| The two do not describe the same coverage in different words | Yes, with #298 | Settings and the popover read one exported string, and a test pins both. The tray card differs but is drawn by the frame (`735:37341`), which CLAUDE.md says wins. |
| The page gives a reason a user would turn on machine-wide routing | **No** | Nothing on the Settings row says why. The reason exists (tools that read proxy variables and have no setting of their own) and is stated only in the OpenCode dialog, which a user who never touches OpenCode never sees. |
| Non-AI traffic is covered only where the user has chosen that, with the benefit stated | **No, and not this PR's** | `proxy_env::export_opted_in` defaults to true: "the routing switch has always implied it. Only an explicit disconnect turns it off." `manager_core.rs` line 330 applies it on every engine enable. So turning routing on routes git and curl machine-wide with no choice made. The OpenCode dialog only fires when the channel is already off, so for a default user it never fires either. |

**Finding 1 (fix here): the row states no reason.** The clause is explicit in the
ticket. Proposed description, replacing the shared sentence's use on the row only,
or extending the shared sentence for both surfaces:

> For tools with no gateway setting of their own. Routes every program you start
> from now on, not only AI tools, and tells Node to trust Gate's certificate.

That is about 150 characters. The current one is 107, and the next-longest
description on the pane is 88; #290's styling pass trimmed a 158-character
version because it wrapped to two lines where no other row does. So the cost is a
two-line description on this one row. Given the ticket asks for the reason in as
many words, two lines is the right trade. Recommend taking it.

**Finding 2 (raise, do not fix): the default contradicts the ticket.** The channel is
on by default and enabled by the master switch. That is a deliberate product call
recorded in `proxy_env.rs` ("the failure that matters is a tool silently not
routing"), it predates the epic, and flipping it would change first-run behaviour
for every user. It also lands on the same question AG-895's write-up parked: how
loud a machine-wide switch should be. The honest state of AG-893 is "three of four
clauses met, the fourth is a default the squad lead has to rule on". The ticket
comment posted today (25085) does not say this and should be amended.

**Residual inaccuracy, minor.** "From now on" follows the repo's own account in
`system_proxy.rs`: launchd is the parent of everything started afterwards, "new
Terminal windows" included. A shell opened in an already-running Terminal.app
inherits Terminal's environment, not launchd's, so it will not see the variables
until Terminal restarts. The old "after your next login" was conservatively true
everywhere and pessimistic on macOS; the new one is optimistic for that one case.
Neither is exact. Not worth a third phrasing; worth knowing if a report comes in.

## AG-887, the hover text

"Gate examined this request and no guardrail matched" on the "Regular" cell.
Accurate for the only case that produces it (`securityAction === "allow"` with no
category, `toolEvents.ts` line 166). Gateway-named categories get no title, which is
right: their spelling is the gateway's and there is nothing to add. The optional
field keeps the `AppPane.test.tsx` fixture compiling. Nothing to change.

## AG-901, the last paragraph

"You are asked this once: turn Claude off whenever you like and the routing stops,
and turning it back on will not ask again." Both halves verified: the section switch
disables the session member with the rest (`useSectionRouting.ts` line 99, sessions
included), and `accept_session_routing` never un-records, so re-enabling does not
re-ask. One statement of the once-only fact. The counting test (`/ask/g` equals 2)
is a little brittle, but it pins exactly the duplication it exists to catch. Nothing
to change.

## The OpenCode dialog, re-read for accuracy

"OpenCode's own settings cover the providers you had set up when you turned it on.
Anything you add later reaches Gate through your machine's proxy variables instead."

- First sentence: `opencode.rs` snapshots the providers present in `opencode.json`
  or `auth.json` at connect, over a fixed allowlist. True.
- Second: a provider added later, or one outside the allowlist, has no `baseURL`
  rewrite and rides `HTTPS_PROXY` into the engine. If Gate knows the host it is
  inspected; if not it is blind-tunnelled. "Reaches Gate" is true in both cases;
  "is inspected" would not be. The sentence does not claim inspection. True.
- The dialog fires before the connect ("asked before the drift gate"), so "when you
  turned it on" is the present moment. Slightly odd tense for a first-time user,
  acceptable.

## Recommended next steps

1. Apply Finding 1 on this branch as a third commit and push.
2. Amend Jira comment 25085 on AG-893 with Finding 2, and add the default-on
   question to the squad lead's list alongside AG-889/AG-897 and AG-895.
3. Nothing else blocks #298.
