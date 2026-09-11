# Routing taxonomy: what a row is

Status: implemented on `feat/routing-taxonomy`, for review.

## Why

The ledger described a routable row with two visible facts and two hidden ones.

| Axis | Where it lived | Visible |
|---|---|---|
| Vendor / model family | the group heading | yes |
| Surface | the row label: "App", "Web", "CLI", "Proxy" | yes |
| Mechanism | `GroupMember.kind: "config" \| "proxy"` | barely |
| Credential | one boolean, `chat` | no |

Four problems followed from that compression, and all four were live.

**1. The row label named a client; the entry was scoped to a host.**
`should_intercept_host` matches on host alone, at CONNECT, before any header
exists. A row labelled "Web" over `claude.ai` therefore also covered the Claude
desktop app - and covered it *more* fully than a browser, since
`rules_for_client` narrows only `ClientClass::Web` to the completion path. A
2026-09 support report read `claude-web off` plus a row saying "Your Claude
chats, in the browser tab" and concluded the desktop app was uncovered. The
taxonomy had no field that could have corrected them.

**2. `chat` was one boolean doing two jobs, and the code said so.**
`provider.rs` carried the admission in its own doc comment: "The name is
historical... The test of membership is the credential, not the protocol." The
`chatgpt` entry rode that field while carrying no chat at all - it is a
subscription bearer on a Responses endpoint.

**3. Excluding a row from a cascade and dropping it from the ledger were one
edit apart.** Membership rode `proxy_domain_slugs`; the exclusion rode a second
array, `chat_domain_slugs`. Keeping a session surface out of the first is what
made those rows invisible for a period, and several paragraphs of comment
existed to warn the next person which array to type a slug into.

**4. Vendor grouping could not file a multi-provider tool**, so four headings
were added by hand - OpenClaw, Hermes, "Experimental", and an `any-provider`
catch-all whose job was to catch what the taxonomy could not place. The
`openai` domain sat under "Experimental" because nothing OpenAI ships rides its
switch.

## What a row is now

Three fields, in `crates/core/src/taxonomy.rs`, shared by the proxy catalog and
the tool registry.

**`Client`** - the program the row is aimed at, and the ledger's grouping key.
`ClaudeCode`, `ClaudeDesktop`, `Codex`, `ChatGpt`, `OpenCode`, `OpenClaw`,
`Hermes`, `AnyApp`. Not `proxy::ClientClass`, which is a per-request reading of
headers; this is a catalog fact.

**`Scope`** - the blast radius. `Host` (every proxy-honouring client on this
machine that talks to these hosts), `Client` (this program only, reached through
something it reads itself), `Machine` (every program started after the next
login).

**`Credential`** - whose key rides the request. `Brokered` (Gate injects from
the keychain, the caller's credential is dropped), `Additive` (the caller's
session cookie or subscription bearer stays, Gate records and inspects),
`Observed` (audited, nothing changed).

Aim and radius are deliberately allowed to disagree. A row aimed at Claude
Desktop with `Scope::Host` covers every client of `claude.ai`, and saying both
is the only honest description of what flipping it does.

## What that changes

**The cascade rule is derived, not listed.** `Provider` has one `domain_slugs`
array; `provider::cascade_domains` filters it by `Credential::cascades()`, and
`cascadeTargets` in `groups.ts` does the same on the frontend. A session surface
can no longer be dropped from the ledger by an edit intended to keep it out of a
cascade, and no new entry can join a cascade by being typed into the wrong
place. Behaviour is unchanged: the derived set is exactly what the old
`proxy_domain_slugs` held, which
`session_domains_stay_listed_while_staying_out_of_the_cascade` now asserts for
every entry rather than for two named ones.

**Scope is rendered.** `scopeNote` gives every host row the sentence it never
had: "Matched on host, so this covers everything on `claude.ai` - whatever on
this machine sends there, not only Chat." Both notes stay browser-neutral;
whether Gate reaches a browser at all is still `browserScopeNote`'s answer,
because on a Linux session with only environment variables written it does not.

**The ledger groups by client.** Eight groups at most, one per client with a
member. `LEFTOVER_GROUPS` and the `any-provider` catch-all are gone - not
unreachable, gone: every row answers `client`, so nothing can fall off. Claude
Code and Claude Desktop are two groups where "Anthropic" was one, which is the
visible cost and also the point: they are two programs with two switches.
`api.openai.com`, OpenRouter and the environment export share "Any app on this
machine", which is what they actually cover.

**Diagnostics and the CLI print the triple.** `gate-connect proxy domains` and
the `[tools]` / `[proxy domains]` sections of the report now carry client, scope
and credential beside the on/off flag. A report saying `claude-web off` alone is
what started this.

## Copy changes that need design sign-off

These are consequences of the heading change, not independent decisions, but
they are user-visible and the Figma is the source of truth on copy:

| Row | Was | Now | Why |
|---|---|---|---|
| `anthropic` | App | API | under a heading reading "Claude Desktop", "App" names nothing |
| `claude-web` | Web | Chat | same heading; and "Web" was the misleading half |
| `chatgpt-apps` | Web | Chat | under "ChatGPT" |
| `chatgpt` | App | Subscription | under "ChatGPT"; names what the endpoint is |
| `openrouter` | Proxy | OpenRouter | heading no longer carries the vendor |
| `opencode` | OpenCode Zen / Go | Zen / Go | heading already says OpenCode |

One description changed on its own merits and is the fix for the support
thread: `claude-web` read "Your Claude chats, in the browser tab" and now reads
"Your Claude chats on claude.ai - in the desktop app, and in a browser tab."

## The rows name the desktop apps on hover

A row's visible text is a surface kind, and a surface kind never names anything
the user could go and open. That is worst for the rows with no config file
behind them: nothing in the app said the word "Cowork" at all, because the
desktop apps have no `list_tools` entry and the row that routes them is
labelled "API".

`MEMBER_HINTS` in `groups.ts` gives each row the programs behind it, rendered as
a native `title` on the label in both the rail and the tray - the same hover
mechanism `AppPane`, `SettingsPane` and `banners` already use. It supplements
the pane's description rather than replacing it, so a viewer that never shows a
`title` loses nothing load-bearing.

| Row | Hover |
|---|---|
| Claude Code > CLI | Claude Code CLI and IDE plugins |
| Claude Desktop > API | Cowork and the Claude desktop app |
| Claude Desktop > Chat | The Claude desktop app, and claude.ai in a browser |
| Codex > CLI | Codex CLI and IDE extension |
| ChatGPT > Chat | The ChatGPT desktop app, and chatgpt.com in a browser |
| ChatGPT > Subscription | Work and the Codex desktop app, on your ChatGPT subscription |

The names are **not** platform-branched: Anthropic's app is Cowork on both
macOS and Windows, OpenAI's is Work on both, confirmed with the product on
2026-09-11. The rows whose subject is a host rather than a product (`openai`,
`openrouter`) deliberately get no hover - there is no program to name, and the
description already says what they cover.

## What this does not do

It does not change what is routed. No host, path, rewrite rule, cascade result
or default enabled flag moves. It is a description of the same behaviour, made
sayable.

It also does not split a host by client - "route the app but not the browser".
That remains impossible to do honestly while interception is decided at CONNECT
from the host alone: any such split still decrypts the client the user did not
opt into, then declines to route it. The taxonomy now makes that constraint
explicit rather than hiding it behind a surface label.

## Open question carried, not resolved

Cowork's client is unsettled in the tree already: `engine.rs:851` validates the
`api.anthropic.com` rewrite "against a real Cowork generation", while
`engine.rs`'s test fixture captures "Cowork's turn" as
`chatgpt.com/backend-api/codex/responses`, and `src-tauri/src/lib.rs`'s
`AGENT_PROCESSES` records that at least one of the two readings is wrong. The
`chatgpt` entry keeps the client the ledger already drew it under rather than
deciding that question here.
