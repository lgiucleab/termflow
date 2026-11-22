//! Byte-stream scanner built on `vte`: extracts shell marks and the printable text between them.

use base64::Engine;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// `OSC 133;A`
    PromptStart,
    /// `OSC 133;B`
    CommandStart,
    /// `OSC 133;C`
    OutputStart,
    /// `OSC 133;D[;exit]`
    Finished(Option<i32>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    Mark(Mark),
    Cwd(String),
    GitBranch(String),
    Command(String),
    /// Alternate screen entered (`true`) or left (`false`), e.g. vim, less, htop.
    AltScreen(bool),
    /// Printable text with escape sequences removed (`\n` kept, `\r` dropped).
    Text(String),
}

/// Feed it PTY bytes in any chunking; it yields the same events.
#[derive(Default)]
pub struct Scanner {
    parser: vte::Parser,
    perf: Collector,
}

#[derive(Default)]
struct Collector {
    text: String,
    events: Vec<ScanEvent>,
}

impl Collector {
    fn push(&mut self, ev: ScanEvent) {
        self.flush_text();
        self.events.push(ev);
    }
    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            self.events.push(ScanEvent::Text(std::mem::take(&mut self.text)));
        }
    }
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ScanEvent> {
        self.parser.advance(&mut self.perf, bytes);
        self.perf.flush_text();
        std::mem::take(&mut self.perf.events)
    }
}

fn utf8(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).and_then(|d| u8::try_from(d).ok())
}

/// Minimal percent-decoding for OSC 7 `file://` URLs.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl vte::Perform for Collector {
    fn print(&mut self, c: char) {
        self.text.push(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => self.text.push('\n'),
            b'\t' => self.text.push('\t'),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        match params {
            [b"133", kind, rest @ ..] => {
                let mark = match kind.first() {
                    Some(b'A') => Mark::PromptStart,
                    Some(b'B') => Mark::CommandStart,
                    Some(b'C') => Mark::OutputStart,
                    Some(b'D') => Mark::Finished(rest.first().and_then(|e| utf8(e).trim().parse().ok())),
                    _ => return,
                };
                self.push(ScanEvent::Mark(mark));
            }
            [b"6973", rest @ ..] if !rest.is_empty() => {
                // Values may contain ';' (vte splits on it), so rejoin before splitting on '='.
                let joined = rest.iter().map(|p| utf8(p)).collect::<Vec<_>>().join(";");
                let Some((key, value)) = joined.split_once('=') else {
                    return;
                };
                let ev = match key {
                    "cwd" => ScanEvent::Cwd(value.to_owned()),
                    "git_branch" => ScanEvent::GitBranch(value.to_owned()),
                    "cmd" => match base64::engine::general_purpose::STANDARD.decode(value.trim()) {
                        Ok(raw) => ScanEvent::Command(utf8(&raw)),
                        Err(_) => return,
                    },
                    _ => return,
                };
                self.push(ev);
            }
            [b"7", rest @ ..] if !rest.is_empty() => {
                let url = rest.iter().map(|p| utf8(p)).collect::<Vec<_>>().join(";");
                if let Some(after) = url.strip_prefix("file://") {
                    // Skip the host part: file://host/path
                    let path = after.find('/').map_or("", |i| &after[i..]);
                    self.push(ScanEvent::Cwd(percent_decode(path)));
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], _ignore: bool, action: char) {
        if intermediates != b"?" || !matches!(action, 'h' | 'l') {
            return;
        }
        if params.iter().any(|p| matches!(p.first(), Some(47 | 1047 | 1049))) {
            self.push(ScanEvent::AltScreen(action == 'h'));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SAMPLE: &[u8] = b"\x1b]6973;cwd=/tmp/a;b\x07\x1b]6973;git_branch=main\x07\x1b]133;A\x07$ \x1b]133;B\x07\
        \x1b]6973;cmd=ZWNobyBoaQ==\x07\x1b]133;C\x07\x1b[31mhi\x1b[0m\r\n\x1b]133;D;1\x07\x1b]7;file://host/Users/me/My%20Dir\x07\
        \x1b[?1049hvim\x1b[?1049l";

    fn all(chunks: &[&[u8]]) -> Vec<ScanEvent> {
        let mut s = Scanner::new();
        let mut out: Vec<ScanEvent> = Vec::new();
        for c in chunks {
            for ev in s.feed(c) {
                // Text may be split differently by chunking; merge adjacent text for comparison.
                if let (ScanEvent::Text(t), Some(ScanEvent::Text(prev))) = (&ev, out.last_mut()) {
                    prev.push_str(t);
                } else {
                    out.push(ev);
                }
            }
        }
        out
    }

    #[test]
    fn parses_all_marks() {
        assert_eq!(
            all(&[SAMPLE]),
            vec![
                ScanEvent::Cwd("/tmp/a;b".into()),
                ScanEvent::GitBranch("main".into()),
                ScanEvent::Mark(Mark::PromptStart),
                ScanEvent::Text("$ ".into()),
                ScanEvent::Mark(Mark::CommandStart),
                ScanEvent::Command("echo hi".into()),
                ScanEvent::Mark(Mark::OutputStart),
                ScanEvent::Text("hi\n".into()),
                ScanEvent::Mark(Mark::Finished(Some(1))),
                ScanEvent::Cwd("/Users/me/My Dir".into()),
                ScanEvent::AltScreen(true),
                ScanEvent::Text("vim".into()),
                ScanEvent::AltScreen(false),
            ]
        );
    }

    #[test]
    fn finished_without_exit_code() {
        assert_eq!(all(&[b"\x1b]133;D\x07"]), vec![ScanEvent::Mark(Mark::Finished(None))]);
    }

    #[test]
    fn ignores_unknown_and_garbage() {
        assert_eq!(
            all(&[b"\x1b]6973;cmd=!!notbase64\x07\x1b]133;Z\x07\x1b]999;x\x07"]),
            vec![]
        );
    }

    proptest! {
        #[test]
        fn chunking_does_not_change_events(cuts in proptest::collection::vec(0..SAMPLE.len(), 0..12)) {
            let mut cuts = cuts;
            cuts.sort_unstable();
            cuts.dedup();
            let mut chunks = Vec::new();
            let mut start = 0;
            for c in cuts { chunks.push(&SAMPLE[start..c]); start = c; }
            chunks.push(&SAMPLE[start..]);
            prop_assert_eq!(all(&chunks), all(&[SAMPLE]));
        }

        #[test]
        fn never_panics_on_random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let _ = Scanner::new().feed(&bytes);
        }
    }
}
