//! The org's compaction window, mapped to each harness's own knob.
//!
//! Gate exposes one setting, `clientConfig.compactionWindow` on every org of
//! `GET /v1/me/orgs` (see [`crate::org::Org::compaction_window`]): "compact
//! the conversation when its context reaches about this many tokens", or
//! `None` for the harness's own default. Gate never compacts anything itself;
//! the lever is client config this app writes, applied by the harness at its
//! next session start. The mapping lives here, in code, so the setting never
//! grows a per-harness surface. The rule for every row is that the value may
//! only LOWER a harness's native trigger, never raise it.
//!
//! | Harness | Knob | Rule |
//! |---|---|---|
//! | Claude Code | `CLAUDE_CODE_AUTO_COMPACT_WINDOW = window + 33000` | 100000-1000000, plain integer; the variable is already capped by the model's context window on the client side, so a 200k model ignores a larger value |
//! | Codex | `model_auto_compact_token_limit` | NOT written: its compaction call is cache-cold today (0% cached on 192/192 prod calls, 18-30% of threads paying more at every threshold). Opt in only after a warm path is shown |
//! | OpenCode | `compaction` block in `opencode.json` | not written: OpenCode's schema is strict and the threshold key is unverified, see [`HarnessKnobs::opencode_compaction`] |
//! | Goose | `GOOSE_AUTO_COMPACT_THRESHOLD` | a percent of the model window, not derivable from a token count alone; no Goose integration exists either |
//! | Hermes, OpenClaw | - | already compact early / prune by cache TTL; no-op |
//!
//! Measured 2026-10-12 on Claude Code: the trigger fires about 33k tokens
//! below the window, so the knob is written as `window + 33000` to make the
//! trigger land at about the window the org asked for. Claude Code parses the
//! variable with `parseInt`, so a suffixed value such as `500k` reads as 500
//! and is clamped up to the 100k floor: the value is always a plain integer.

/// Claude Code's floor for `CLAUDE_CODE_AUTO_COMPACT_WINDOW`; anything below
/// it is clamped up by the tool.
pub const CLAUDE_CODE_WINDOW_MIN: u32 = 100_000;
/// Claude Code's ceiling for `CLAUDE_CODE_AUTO_COMPACT_WINDOW`.
pub const CLAUDE_CODE_WINDOW_MAX: u32 = 1_000_000;
/// How far below the variable Claude Code's auto-compaction fires.
pub const CLAUDE_CODE_TRIGGER_MARGIN: u32 = 33_000;

/// Every harness knob derived from one window, `None` where that harness gets
/// nothing. Only Claude Code's is ever `Some` in this round; the other fields
/// exist so the table above has one place in code that says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessKnobs {
    /// The value of `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, already rendered as the
    /// plain-integer string the `settings.json` env block carries.
    pub claude_code_auto_compact_window: Option<String>,
    /// Codex's `model_auto_compact_token_limit`. Always `None`: see the table.
    pub codex_auto_compact_token_limit: Option<u32>,
    /// OpenCode's `compaction` threshold. Always `None`: `opencode.json` is
    /// validated against a strict schema (`additionalProperties: false`, see
    /// `integrations::opencode`), and the key that would carry a token
    /// threshold has not been verified against it, so writing one could make
    /// OpenCode reject its own config file.
    pub opencode_compaction: Option<u32>,
    /// Goose's `GOOSE_AUTO_COMPACT_THRESHOLD`, a fraction of the model's
    /// context window. Always `None`: the fraction needs the window of the
    /// model in use, which this app does not know.
    pub goose_auto_compact_threshold: Option<u32>,
}

/// Map the org's window to every harness knob. `None` in means every knob
/// stays at the harness default.
pub fn harness_knobs(window: Option<u32>) -> HarnessKnobs {
    HarnessKnobs {
        claude_code_auto_compact_window: window.map(|w| {
            w.saturating_add(CLAUDE_CODE_TRIGGER_MARGIN)
                .clamp(CLAUDE_CODE_WINDOW_MIN, CLAUDE_CODE_WINDOW_MAX)
                .to_string()
        }),
        codex_auto_compact_token_limit: None,
        opencode_compaction: None,
        goose_auto_compact_threshold: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_window_means_no_knob_anywhere() {
        let knobs = harness_knobs(None);
        assert_eq!(knobs.claude_code_auto_compact_window, None);
        assert_eq!(knobs.codex_auto_compact_token_limit, None);
        assert_eq!(knobs.opencode_compaction, None);
        assert_eq!(knobs.goose_auto_compact_threshold, None);
    }

    #[test]
    fn claude_code_gets_the_window_plus_the_trigger_margin_as_a_plain_integer() {
        assert_eq!(
            harness_knobs(Some(400_000)).claude_code_auto_compact_window,
            Some("433000".to_string())
        );
        // Never a suffixed form: Claude Code would read "433k" as 433.
        let rendered = harness_knobs(Some(600_000))
            .claude_code_auto_compact_window
            .unwrap();
        assert!(rendered.bytes().all(|b| b.is_ascii_digit()), "{rendered}");
    }

    #[test]
    fn claude_code_window_is_clamped_to_the_tool_range() {
        // The ceiling: 1000000 + 33000 would be out of range.
        assert_eq!(
            harness_knobs(Some(1_000_000)).claude_code_auto_compact_window,
            Some("1000000".to_string())
        );
        // The floor is within range once the margin is added.
        assert_eq!(
            harness_knobs(Some(100_000)).claude_code_auto_compact_window,
            Some("133000".to_string())
        );
        // A value below the floor (not one Gate hands out, but the clamp is
        // what makes the knob safe to write regardless) is raised to it.
        assert_eq!(
            harness_knobs(Some(1)).claude_code_auto_compact_window,
            Some("100000".to_string())
        );
        assert_eq!(
            harness_knobs(Some(u32::MAX)).claude_code_auto_compact_window,
            Some("1000000".to_string())
        );
    }

    /// Codex is deliberately not mapped this round: its compaction call is
    /// cache-cold, so an earlier trigger costs more on 18-30% of threads.
    #[test]
    fn codex_opencode_and_goose_get_nothing_for_any_window() {
        for window in [Some(100_000), Some(400_000), Some(1_000_000)] {
            let knobs = harness_knobs(window);
            assert_eq!(knobs.codex_auto_compact_token_limit, None);
            assert_eq!(knobs.opencode_compaction, None);
            assert_eq!(knobs.goose_auto_compact_threshold, None);
        }
    }
}
