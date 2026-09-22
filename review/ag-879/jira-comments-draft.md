# Jira comments after #290 (POSTED 2026-09-17 15:32 UTC-0, comment ids 25083-25089)

Posted from this file on 2026-09-17: AG-887 (25083), AG-891 (25084), AG-893 (25085), AG-883 (25086), AG-901 (25087), AG-881 (25088), AG-900 (25089). Plain text, American spelling, no em dashes. Each is
ready to paste into the ticket named. Suggested status moves are in brackets
and are not part of the comment text.

## For you to decide first

1. **AG-900** is assigned to Gabriel Claramunt and In Progress, and #290 fixed
   it. Confirm with Gabriel that they have no AG-900 branch of their own before
   moving it to In review, as happened with AG-880 and AG-886.
2. **AG-888**: your Jira comment says the set is an allow-list and single-select
   would revert AG-746. Gabriel's draft #291 "Offer one model where the app runs
   one (AG-888)" does the single-select. Both cannot land. The ticket is Blocked;
   the two of you need one answer before either PR moves.
3. **Error banner under the scrim** (`aea4062` in #290) reverts #244 and is not
   on any AG-879 ticket. The commit names the follow-up (report a dialog's own
   failure inside the dialog) and does not do it. File it; under AG-879 or
   AG-550, your call.
4. **AG-882** is Blocked with your close-or-rescope recommendation. Nobody has
   filed the gateway ticket the comment proposes (Connect's `tokensSaved` is
   compression-only while every dashboard surface plots Gate-attributed
   savings). File it or ask Gabriel to.
5. **AG-889** carries the same comment twice, both dated 2026-09-16. Delete the
   shorter one.
6. Optional pre-close check for AG-891: on staging with a pasted API key rather
   than OAuth, confirm what `/v1/me/credits` returns. If it refuses API-key
   auth, Settings now shows "Unavailable" with a Retry that cannot succeed for
   those users.

## AG-887 [move to In review]

Merged in #290. The Type column is the guardrail category the gateway records
for a request, which is what the frame draws there and what the per-tool
events payload carries. That payload has no per-request "type" in the sense
of chat versus completion versus tool call: each event carries a time, a
status, a security action (allow, flag, redact or block), a security category,
a model, a provider and a session reference, and nothing else. So the column
cannot show a request type, and the epic's rule against inventing vocabulary
the gateway does not have applies.

What was wrong is that ordinary traffic carried no category, so every row drew
a dash and read as missing. A request the gateway examined and allowed with
nothing matching now reads "Regular". A row with no security action at all
keeps the dash, because we have no reading for it. A block, flag or redact
with no category also keeps the dash rather than being called Regular.

Does that answer the report, or were you expecting a request type the payload
would need to grow first? If the latter, that is a gateway ticket.

## AG-891 [move to In review, do not close]

Merged in #290. Settings now reads the plan from the same `/v1/me/credits`
payload the app pane uses and shows "Pro" for the gateway's `paid`, which is
the word the dashboard uses in its sidebar, billing page and emails. The row
has four states: a skeleton while the read is in flight, the plan, a dash when
the read landed and named no plan, and "Unavailable" with a Retry when the
read failed.

Do not close this yet. The endpoint reads a cached entitlement column rather
than the resolver, so a lapsed org still reports `paid` and Connect would name
a plan the user no longer holds. Constellation-Labs/gate#1043 fixes that on the
gateway side. This ticket closes when that lands.

## AG-893 [move to In review after the follow-up PR]

Merged in #290, with a copy correction in a follow-up PR. The two controls this
ticket describes are gone: #273 removed the "Also set shell environment
variables" card, and #290 removed the Terminal row from the app list, because
a machine-wide setting is not an app. The one control is now a "Command-line
tools" toggle in Settings, under Connection. Its description says what it
reaches (every program you start from now on, not only AI tools) and what it
costs (Node is told to trust Gate's certificate). The menu bar popover shows
the same state as a reading and offers no second switch. The OpenCode dialog
that turns the channel on when you turn OpenCode on says why: OpenCode's own
settings cover the providers you had set up, and the channel carries anything
added later.

On the "why would I want git and curl going through Gate" question in the
report: you would not, and the copy no longer suggests a benefit. Gate
inspects only the AI providers it knows and passes everything else through
untouched, so routing git and curl changes none of their traffic. The reason
to turn it on is coverage for tools that read proxy variables and have no
setting of their own.

## AG-883 [move to In review]

Merged in #290. Measured in the real window: nothing was being inserted, the
pane was pinning at its maximum scroll because Token savings is the
second-to-last card and there was nothing below it to fill the view. The tile
now navigates to the Token savings section only when that section has rows to
land on, and is an ordinary tile otherwise, including while the read is in
flight or after it failed.

This keeps AG-572's requirement that the counter navigates. If you meant that
clicking the tile should never move the page, say so and it becomes a plain
tile in every state.

## AG-901 [move to In review]

Merged in #290. The dialog now leads with what the switch does and names the
host, then says what Gate records and what it does not (it sees nothing you
were not already sending, and passes your existing login through rather than
signing in for you), then that everything on this machine sending to that host
goes the same way, then that the question is asked once.

On "how to change the decision later": the dialog says turning the app off
stops the routing, and does not point at Settings. That is deliberate. Nothing
in Settings changes this answer, by design: the acceptance is recorded once so
that turning the app back on does not ask again, and a reset removes
credentials but leaves that preference in place. Copy pointing at Settings
would promise something the app does not do.

## AG-881 [move to In review, no comment needed]

The ticket's Expected is exactly what was built. If a comment is wanted:
"Merged in #290. The tile shows the savings rate only. The gateway still sends
the amount and currency; Connect no longer reads them."

## AG-900 [after confirming with Gabriel, move to In review]

Merged in #290. The reopen offer was built from the members that write a
config file, so of the Claude section's three members only Claude Code
survived and the desktop app was never asked about. Every moved member is now
passed and the backend drops the ones with no process, so closing Claude now
offers both Claude Code and the Claude desktop app, and reports each one's
state separately. The e2e harness could not model a running desktop app at
all, which is why nothing caught it; it can now.

One interaction to know about: the same change means the ChatGPT / Codex
switch offers to close the ChatGPT desktop app too, so the reopen dialog can
show ChatGPT and Codex as two rows. #288 (AG-898) collapses that view to one
row per app; whichever lands second should be checked against the other.
