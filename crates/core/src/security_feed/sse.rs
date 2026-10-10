//! Server-Sent Events framing, per the WHATWG event-stream grammar.
//!
//! Hand-rolled rather than pulled from a crate. `eventsource-client` would bring
//! a second HTTP stack into a tree that already carries `reqwest` and `hyper`,
//! and the part we actually need - the line grammar - is small, fully specified,
//! and has a reference implementation to test against in `@gate/sse`
//! (`parseSseEvents`) in the `gate` repo. What a crate would buy is connection
//! management, and that is the part we deliberately write ourselves: the backoff
//! and the credential rules are ours, not a library's.
//!
//! The grammar, and the reasons each rule is here rather than "obvious":
//!
//! - A frame ends at a blank line. Anything after the last blank line is a
//!   partial frame and must stay buffered - the network splits wherever it likes,
//!   and a chunk boundary in the middle of `data:` is the common case, not the
//!   edge case.
//! - Line terminators are `\r\n`, `\n` **or** a bare `\r`. All three are legal
//!   and a server behind a proxy that rewrites them is not a bug we get to fix.
//! - `field: value` strips exactly one leading space from the value, not all
//!   whitespace. A payload that legitimately begins with a space would otherwise
//!   come back altered.
//! - A line with no colon is a field with an empty value.
//! - A line starting with `:` is a comment. The 15s keepalive is exactly this,
//!   so discarding them is load-bearing, not tidiness.
//! - Multiple `data:` lines in one frame join with `\n`, with no trailing
//!   newline. A single `data:` frame is the common case but not the contract.
//! - `id:` containing a NUL is ignored per spec, and the id persists across
//!   frames until changed - it is a stream position, not a frame field.
//! - A leading UTF-8 BOM is stripped once, at the start of the stream only.

/// One decoded event. `event` is absent when the frame carried no `event:` line;
/// the spec's default is `message`, but resolving that here would hide from the
/// caller whether the server named the type, so it stays `None`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Event {
    pub event: Option<String>,
    pub data: String,
    /// The stream position after this event, if the stream has one yet.
    pub id: Option<String>,
    /// The server's requested reconnection floor, in milliseconds.
    pub retry: Option<u64>,
}

/// Incremental decoder. Feed it bytes as they arrive; take whole events out.
///
/// Holds the last seen `id` because the spec makes it stream state rather than
/// frame state: a server that sends `id:` once and then twenty events expects
/// all twenty to resume from it.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    last_id: Option<String>,
    started: bool,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The position to resume from, for `Last-Event-ID`.
    pub fn last_id(&self) -> Option<&str> {
        self.last_id.as_deref()
    }

    /// Seed the position from a previous connection, so a reconnect does not
    /// have to wait for the server to re-send an id before it can resume again.
    pub fn seed_last_id(&mut self, id: Option<String>) {
        self.last_id = id;
    }

    /// Push received bytes and drain whatever complete events they finished.
    ///
    /// Returns events in stream order. Frames that decode to nothing - a
    /// keepalive comment, a frame of only unknown fields - yield no event, which
    /// is why this returns a `Vec` rather than an `Option`.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(chunk);
        if !self.started {
            // Only ever at the very start of the stream, and only once: a BOM
            // appearing mid-stream is data, not a marker.
            if self.buf.len() >= 3 {
                if self.buf.starts_with(&[0xEF, 0xBB, 0xBF]) {
                    self.buf.drain(..3);
                }
                self.started = true;
            } else if !self.buf.is_empty() && self.buf[0] != 0xEF {
                self.started = true;
            }
        }

        let mut out = Vec::new();
        while let Some((end, rest)) = split_frame(&self.buf) {
            let frame: Vec<u8> = self.buf[..end].to_vec();
            self.buf.drain(..rest);
            if let Some(ev) = self.decode_frame(&frame) {
                out.push(ev);
            }
        }
        out
    }

    fn decode_frame(&mut self, frame: &[u8]) -> Option<Event> {
        let mut data: Vec<String> = Vec::new();
        let mut event: Option<String> = None;
        let mut retry: Option<u64> = None;
        let mut saw_field = false;

        for line in split_lines(frame) {
            let line = String::from_utf8_lossy(line);
            if line.is_empty() {
                continue;
            }
            if line.starts_with(':') {
                // Comment, including the keepalive. Not an event, and not a
                // reason to emit an empty one.
                continue;
            }
            let (field, value) = match line.find(':') {
                Some(i) => {
                    let v = &line[i + 1..];
                    // Exactly one leading space, per spec.
                    (&line[..i], v.strip_prefix(' ').unwrap_or(v))
                }
                None => (&line[..], ""),
            };
            saw_field = true;
            match field {
                "data" => data.push(value.to_string()),
                "event" => event = Some(value.to_string()),
                "id" => {
                    // A NUL in the id is ignored outright rather than sanitised:
                    // resuming from a mangled position is worse than resuming
                    // from the previous one.
                    if !value.contains('\0') {
                        self.last_id = Some(value.to_string());
                    }
                }
                "retry" => {
                    if let Ok(ms) = value.parse::<u64>() {
                        retry = Some(ms);
                    }
                }
                // Unknown fields are ignored, per spec, so a server can add one
                // without breaking this client.
                _ => {}
            }
        }

        // A frame that carried only an `id:` or only unknown fields advances the
        // stream position but is not an event to hand up.
        if !saw_field || (data.is_empty() && event.is_none() && retry.is_none()) {
            return None;
        }

        Some(Event {
            event,
            data: data.join("\n"),
            id: self.last_id.clone(),
            retry,
        })
    }
}

/// Find the first complete frame, as `(frame_end, resume_at)` byte offsets.
///
/// A frame ends at a blank line, which is any of `\n\n`, `\r\n\r\n` or `\r\r`.
/// Searching for all three rather than normalising the buffer first avoids
/// rewriting it on every chunk.
///
/// Offsets rather than slices so the caller can replace the buffer it is
/// draining: returning borrows of `self.buf` would pin it for the assignment
/// that consumes them.
fn split_frame(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < buf.len() {
        if buf[i..].starts_with(b"\r\n\r\n") {
            return Some((i, i + 4));
        }
        if buf[i..].starts_with(b"\n\n") {
            return Some((i, i + 2));
        }
        if buf[i..].starts_with(b"\r\r") {
            return Some((i, i + 2));
        }
        i += 1;
    }
    None
}

/// Split a frame into lines on any of the three legal terminators.
fn split_lines(frame: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < frame.len() {
        if frame[i] == b'\r' {
            lines.push(&frame[start..i]);
            // `\r\n` is one terminator, not two.
            i += if frame[i..].starts_with(b"\r\n") {
                2
            } else {
                1
            };
            start = i;
        } else if frame[i] == b'\n' {
            lines.push(&frame[start..i]);
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < frame.len() {
        lines.push(&frame[start..]);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(d: &mut Decoder, s: &str) -> Vec<Event> {
        d.push(s.as_bytes())
    }

    #[test]
    fn decodes_a_simple_frame() {
        let mut d = Decoder::new();
        let ev = one(&mut d, "event: hello\ndata: {\"a\":1}\n\n");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].event.as_deref(), Some("hello"));
        assert_eq!(ev[0].data, "{\"a\":1}");
    }

    #[test]
    fn holds_a_partial_frame_until_it_completes() {
        let mut d = Decoder::new();
        assert!(one(&mut d, "event: security-event\ndata: {\"requ").is_empty());
        let ev = one(&mut d, "estId\":\"x\"}\n\n");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].data, "{\"requestId\":\"x\"}");
    }

    #[test]
    fn splits_a_chunk_carrying_several_frames() {
        let mut d = Decoder::new();
        let ev = one(&mut d, "data: one\n\ndata: two\n\ndata: three\n\n");
        assert_eq!(ev.len(), 3);
        assert_eq!(ev[2].data, "three");
    }

    #[test]
    fn joins_multiline_data_with_newlines_and_no_trailer() {
        let mut d = Decoder::new();
        let ev = one(&mut d, "data: a\ndata: b\ndata: c\n\n");
        assert_eq!(ev[0].data, "a\nb\nc");
    }

    #[test]
    fn accepts_crlf_and_bare_cr_terminators() {
        let mut d = Decoder::new();
        assert_eq!(one(&mut d, "data: crlf\r\n\r\n")[0].data, "crlf");
        assert_eq!(one(&mut d, "data: cr\r\r")[0].data, "cr");
    }

    #[test]
    fn strips_exactly_one_leading_space() {
        let mut d = Decoder::new();
        // Two spaces in, one space out: the second is the payload's own.
        assert_eq!(one(&mut d, "data:  padded\n\n")[0].data, " padded");
        assert_eq!(one(&mut d, "data:tight\n\n")[0].data, "tight");
    }

    #[test]
    fn discards_keepalive_comments_without_emitting() {
        let mut d = Decoder::new();
        assert!(one(&mut d, ":\n\n").is_empty());
        assert!(one(&mut d, ": keepalive\n\n").is_empty());
        // And a comment sharing a frame with real data does not suppress it.
        assert_eq!(one(&mut d, ": ping\ndata: real\n\n")[0].data, "real");
    }

    #[test]
    fn carries_the_id_forward_across_frames() {
        let mut d = Decoder::new();
        let ev = one(&mut d, "id: 01J\ndata: first\n\ndata: second\n\n");
        assert_eq!(ev[0].id.as_deref(), Some("01J"));
        // The position is stream state, so the second event resumes from it too.
        assert_eq!(ev[1].id.as_deref(), Some("01J"));
        assert_eq!(d.last_id(), Some("01J"));
    }

    #[test]
    fn ignores_an_id_containing_nul() {
        let mut d = Decoder::new();
        one(&mut d, "id: good\n\n");
        one(&mut d, "id: b\0ad\ndata: x\n\n");
        assert_eq!(d.last_id(), Some("good"));
    }

    #[test]
    fn parses_retry_and_ignores_a_non_numeric_one() {
        let mut d = Decoder::new();
        assert_eq!(one(&mut d, "retry: 2500\ndata: x\n\n")[0].retry, Some(2500));
        assert_eq!(one(&mut d, "retry: soon\ndata: x\n\n")[0].retry, None);
    }

    #[test]
    fn treats_a_colonless_line_as_an_empty_field() {
        let mut d = Decoder::new();
        // `data` alone is a data line with an empty value, so the frame is real.
        assert_eq!(one(&mut d, "data\n\n")[0].data, "");
    }

    #[test]
    fn ignores_unknown_fields_without_dropping_the_frame() {
        let mut d = Decoder::new();
        let ev = one(&mut d, "future: whatever\ndata: kept\n\n");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].data, "kept");
    }

    #[test]
    fn strips_a_leading_bom_once() {
        let mut d = Decoder::new();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"data: x\n\n");
        let ev = d.push(&bytes);
        assert_eq!(ev[0].data, "x");
    }

    #[test]
    fn survives_a_split_inside_a_crlf_terminator() {
        let mut d = Decoder::new();
        assert!(d.push(b"data: split\r").is_empty());
        let ev = d.push(b"\n\r\n");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].data, "split");
    }

    #[test]
    fn a_frame_of_only_an_id_advances_position_without_an_event() {
        let mut d = Decoder::new();
        assert!(one(&mut d, "id: 01K\n\n").is_empty());
        assert_eq!(d.last_id(), Some("01K"));
    }

    #[test]
    fn seeded_position_survives_a_reconnect() {
        let mut d = Decoder::new();
        d.seed_last_id(Some("01PREV".into()));
        assert_eq!(d.last_id(), Some("01PREV"));
        // And an event decoded before the server re-sends an id reports it.
        assert_eq!(one(&mut d, "data: x\n\n")[0].id.as_deref(), Some("01PREV"));
    }
}
