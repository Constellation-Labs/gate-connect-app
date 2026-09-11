//! What a routable row *is*, as three independent facts.
//!
//! The ledger used to carry two of these implicitly and one not at all. A row
//! had a vendor (its family heading), a surface label ("App", "Web", "CLI"),
//! a mechanism squeezed into `GroupMember.kind`, and a single `chat` boolean
//! standing in for the credential relationship. That compression is what made
//! three separate questions unanswerable on screen:
//!
//! - **Who sends this traffic?** The surface label named a client ("App"), but
//!   the entry behind it is scoped to a HOST, so the label promised a
//!   distinction the engine does not make at CONNECT time. A user reading
//!   "Web" concluded their desktop app was not covered when it was, and more
//!   fully than the browser (see [`Scope::Host`]).
//! - **How does Gate get in front of it?** Config rewrite, host interception,
//!   the loopback relay and the machine-wide environment export fail in
//!   completely different ways, and the user was told none of them.
//! - **Whose credential rides the request?** The product promise, carried by a
//!   boolean whose name said "protocol" while its doc said "credential".
//!
//! So: [`Client`] is who a row is aimed at, [`Scope`] is how wide its blast
//! radius actually is, and [`Credential`] is whose key goes on the wire. The
//! first two are deliberately allowed to disagree - a row aimed at Claude
//! Desktop whose scope is `Host` covers every client of `claude.ai`, and
//! saying both is the only honest description of what flipping it does.

use serde::{Deserialize, Serialize};

/// Whose credential ends up on the request, which is the whole product promise
/// and the reason the family switch cannot be a hand-kept list of slugs.
///
/// This replaces `Provider::chat_domain_slugs`, a second array whose only job
/// was to hold the surfaces the family switch must never flip. Keeping that
/// list by hand is what the old comments spend paragraphs warning about: add a
/// session-cookie surface to the wrong array and enabling "Claude" starts
/// routing the user's signed-in identity. Now the fact that decides it lives on
/// the row, and [`Credential::cascades`] derives the rule from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Credential {
    /// Gate injects the key (or Cognito token) it holds in the OS keychain, and
    /// the caller's own credential is dropped on the rewrite path. The only
    /// kind a family switch may turn on, because it is the only kind where the
    /// thing being routed is a credential Gate brokers rather than one the user
    /// is already signed in with.
    Brokered,
    /// The caller's own session cookie or subscription bearer stays on the
    /// request and Gate adds its headers alongside. `claude.ai`'s chat turn and
    /// `chatgpt.com`'s conversation endpoint are these: Gate records and
    /// inspects the traffic rather than supplying a key for it. Always opt-in,
    /// per row, never by cascade.
    Additive,
    /// Gate sees the request and writes it to the audit trail without changing
    /// what authenticates it. No entry ships this today; it exists because
    /// "inspected" and "credential swapped" are different promises, and a row
    /// that only watches must be able to say so rather than borrowing
    /// [`Credential::Additive`]'s sentence.
    Observed,
}

impl Credential {
    /// Whether a family switch may flip a row with this credential.
    ///
    /// The one place the rule lives. `provider::enable` and `disable` filter on
    /// it, the UI reads the answer off the row rather than re-deriving it, and
    /// `provider::cascade_members` is the test that pins the two together.
    pub const fn cascades(self) -> bool {
        matches!(self, Credential::Brokered)
    }
}

/// How wide the blast radius is when this row is switched on.
///
/// Named for coverage, not for timing: the question it answers is "what else on
/// my machine does this touch", which is the one the surface label could not
/// answer and the one that generated the support thread this taxonomy came
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Every client on this machine that honours the system proxy and talks to
    /// these hosts.
    ///
    /// This is the value that must never be hidden. `proxy::should_intercept_host`
    /// matches on host alone and runs at CONNECT, before any header exists, so
    /// a row aimed at one app still decrypts the same host for every other
    /// client. The per-request narrowing that follows
    /// (`proxy::rules_for_client`) decides what is REWRITTEN, never what is
    /// intercepted.
    Host,
    /// This program only. Gate reached it through something the program itself
    /// reads: its config file, a relay base URL, or a route selector carried in
    /// its proxy URL. Nothing else on the machine changes.
    Client,
    /// Every program started after the next login. The environment export
    /// writes machine-wide settings, so it reaches `git` and `curl` and not
    /// only AI tools.
    Machine,
}

/// Who a row is aimed at: the program on the user's machine whose traffic it
/// exists to route.
///
/// **Not [`crate::proxy::ClientClass`]**, and the two must not be confused.
/// That one is a per-request reading of the headers on an intercepted
/// connection, deciding whether the bytes in hand came from a browser or an
/// app. This one is a catalog fact, fixed when the entry is written, and it is
/// what the ledger groups by.
///
/// Grouping by this rather than by vendor is what retires four headings. The
/// vendor grouping could not file a tool that routes whatever providers the
/// user configured in it, so OpenClaw, Hermes, an "Experimental" pair and an
/// `any-provider` catch-all each got a heading of their own - and the catch-all
/// existed precisely to catch what the taxonomy could not place. Every row has
/// exactly one client, so nothing can fall off the ledger and the catch-all is
/// gone by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Client {
    ClaudeCode,
    /// The Claude desktop app. Two rows are aimed at it, and that is the point
    /// of naming the client rather than the surface: `api.anthropic.com` for
    /// its inference calls and `claude.ai` for its chat turns are two surfaces
    /// of one program, and a user who wants "my Claude app routed" wants both.
    ClaudeDesktop,
    Codex,
    /// The ChatGPT desktop app, and the browser tab beside it on the same host.
    ChatGpt,
    OpenCode,
    OpenClaw,
    Hermes,
    /// Not one program: rows that cover whatever on the machine happens to talk
    /// to them. `api.openai.com`, OpenRouter and the environment export are
    /// these. It is a real answer rather than a leftover bin - "anything here
    /// that reaches this host" is exactly what those rows do - which is why it
    /// carries no vendor and sorts last.
    AnyApp,
}

impl Client {
    /// Stable identifier: the ledger group id, and what the CLI prints.
    pub const fn slug(self) -> &'static str {
        match self {
            Client::ClaudeCode => "claude-code",
            Client::ClaudeDesktop => "claude-desktop",
            Client::Codex => "codex",
            Client::ChatGpt => "chatgpt",
            Client::OpenCode => "opencode",
            Client::OpenClaw => "openclaw",
            Client::Hermes => "hermes",
            Client::AnyApp => "any-app",
        }
    }

    /// The group heading. A product name, because that is what the user
    /// recognises on their own machine; the rows underneath carry the surface.
    pub const fn display_name(self) -> &'static str {
        match self {
            Client::ClaudeCode => "Claude Code",
            Client::ClaudeDesktop => "Claude Desktop",
            Client::Codex => "Codex",
            Client::ChatGpt => "ChatGPT",
            Client::OpenCode => "OpenCode",
            Client::OpenClaw => "OpenClaw",
            Client::Hermes => "Hermes",
            Client::AnyApp => "Any app on this machine",
        }
    }

    /// The vendor whose models this client talks to natively, for ordering and
    /// for the rail's caption.
    ///
    /// `None` where there isn't one, and the absence is a fact rather than a
    /// gap: OpenCode, OpenClaw and Hermes route whatever providers the user
    /// configured in them, and [`Client::AnyApp`] is not a program at all. The
    /// old ledger called that "your existing providers" and then had to explain
    /// that the phrase was not a caption.
    pub const fn vendor(self) -> Option<&'static str> {
        match self {
            Client::ClaudeCode | Client::ClaudeDesktop => Some("Anthropic"),
            Client::Codex | Client::ChatGpt => Some("OpenAI"),
            Client::OpenCode | Client::OpenClaw | Client::Hermes | Client::AnyApp => None,
        }
    }

    /// Every client, in the order the ledger draws them: vendor families first
    /// in catalog order, then the tools with no single vendor, then the
    /// machine-wide row last because it is the widest and the least specific.
    pub const ALL: [Client; 8] = [
        Client::ClaudeCode,
        Client::ClaudeDesktop,
        Client::Codex,
        Client::ChatGpt,
        Client::OpenCode,
        Client::OpenClaw,
        Client::Hermes,
        Client::AnyApp,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cascade rule, pinned where it is defined rather than only where it
    /// is applied. `Brokered` and only `Brokered` may ride a family switch:
    /// the other two carry a credential the user is already signed in with,
    /// and routing that is a deliberate per-row act.
    #[test]
    fn only_brokered_credentials_cascade() {
        assert!(Credential::Brokered.cascades());
        assert!(!Credential::Additive.cascades());
        assert!(!Credential::Observed.cascades());
    }

    /// Slugs are ids - they reach config, the CLI and the ledger's group keys -
    /// so two clients answering the same one would merge two groups into one
    /// silently.
    #[test]
    fn client_slugs_are_distinct() {
        let mut seen: Vec<&str> = Vec::new();
        for client in Client::ALL {
            assert!(
                !seen.contains(&client.slug()),
                "{} is claimed twice",
                client.slug()
            );
            seen.push(client.slug());
        }
    }

    /// `ALL` is the ledger's draw order, so a client missing from it is a
    /// heading that never renders. Nothing else enumerates the enum, which is
    /// why this has to.
    #[test]
    fn all_holds_every_client() {
        // Exhaustive match: adding a variant fails to compile here until it is
        // added to `ALL` below it.
        for client in Client::ALL {
            match client {
                Client::ClaudeCode
                | Client::ClaudeDesktop
                | Client::Codex
                | Client::ChatGpt
                | Client::OpenCode
                | Client::OpenClaw
                | Client::Hermes
                | Client::AnyApp => {}
            }
        }
        assert_eq!(Client::ALL.len(), 8);
    }

    /// The one client that is not a program carries no vendor, and the ones
    /// that are a vendor's own product do.
    #[test]
    fn vendors_are_set_only_where_there_is_one() {
        assert_eq!(Client::ClaudeDesktop.vendor(), Some("Anthropic"));
        assert_eq!(Client::ChatGpt.vendor(), Some("OpenAI"));
        assert_eq!(Client::OpenClaw.vendor(), None);
        assert_eq!(Client::AnyApp.vendor(), None);
    }
}
