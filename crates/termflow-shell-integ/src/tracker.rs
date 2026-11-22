//! Turns scanner events into blocks. Lenient about repeated marks; gives up (degrades) on
//! orderings that can't be trusted, e.g. two output starts in a row.

use jiff::Timestamp;
use termflow_types::{Block, BlockId, TerminalId};

use crate::scanner::{Mark, ScanEvent};

/// Plain-text output kept per block for agent context (tail is kept).
pub const EXCERPT_CAP: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackerEvent {
    Started(Block),
    Finished { block: Block, output: String },
    Degraded(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No prompt seen yet, or between `D` and the next `A`.
    Idle,
    Prompt,
    Input,
    Running,
    Degraded,
}

pub struct BlockTracker {
    terminal_id: TerminalId,
    shell: String,
    state: State,
    seq: u32,
    cwd: Option<String>,
    git_branch: Option<String>,
    pending_cmd: Option<String>,
    current: Option<Block>,
    output: String,
    alt_screen: bool,
}

impl BlockTracker {
    pub fn new(terminal_id: TerminalId, shell: impl Into<String>) -> Self {
        Self {
            terminal_id,
            shell: shell.into(),
            state: State::Idle,
            seq: 0,
            cwd: None,
            git_branch: None,
            pending_cmd: None,
            current: None,
            output: String::new(),
            alt_screen: false,
        }
    }

    pub fn is_degraded(&self) -> bool {
        self.state == State::Degraded
    }

    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    pub fn on(&mut self, ev: ScanEvent, now: Timestamp) -> Option<TrackerEvent> {
        if self.state == State::Degraded {
            return None;
        }
        match ev {
            ScanEvent::Cwd(c) => self.cwd = Some(c),
            ScanEvent::GitBranch(b) => self.git_branch = (!b.is_empty()).then_some(b),
            ScanEvent::Command(c) => self.pending_cmd = Some(c),
            ScanEvent::AltScreen(on) => self.alt_screen = on,
            ScanEvent::Text(t) => {
                if self.state == State::Running && !self.alt_screen {
                    self.push_output(&t);
                }
            }
            ScanEvent::Mark(m) => return self.on_mark(m, now),
        }
        None
    }

    fn on_mark(&mut self, mark: Mark, now: Timestamp) -> Option<TrackerEvent> {
        match (mark, self.state) {
            // A prompt while a command is "running" means D was lost: close it without an exit code.
            (Mark::PromptStart, State::Running) => {
                let ev = self.finish(None, now);
                self.state = State::Prompt;
                ev
            }
            (Mark::PromptStart, _) => {
                self.state = State::Prompt;
                None
            }
            (Mark::CommandStart, State::Idle | State::Prompt | State::Input) => {
                self.state = State::Input;
                None
            }
            (Mark::OutputStart, State::Prompt | State::Input) => {
                let block = Block {
                    id: BlockId::new(),
                    terminal_id: self.terminal_id,
                    seq: self.seq,
                    command: self.pending_cmd.take().unwrap_or_default().trim_end().to_owned(),
                    cwd: self.cwd.clone(),
                    git_branch: self.git_branch.clone(),
                    shell: self.shell.clone(),
                    exit_code: None,
                    started_at: now,
                    finished_at: None,
                    chat_session_id: None,
                };
                self.seq += 1;
                self.output.clear();
                self.alt_screen = false;
                self.current = Some(block.clone());
                self.state = State::Running;
                Some(TrackerEvent::Started(block))
            }
            (Mark::OutputStart, state) => {
                self.state = State::Degraded;
                self.current = None;
                Some(TrackerEvent::Degraded(format!("output-start mark while {state:?}")))
            }
            (Mark::Finished(code), State::Running) => {
                let ev = self.finish(code, now);
                self.state = State::Idle;
                ev
            }
            // Repeated or stray marks (e.g. fish's own OSC 133 alongside ours): ignore.
            (Mark::CommandStart | Mark::Finished(_), _) => None,
        }
    }

    fn finish(&mut self, exit_code: Option<i32>, now: Timestamp) -> Option<TrackerEvent> {
        let mut block = self.current.take()?;
        block.exit_code = exit_code;
        block.finished_at = Some(now);
        Some(TrackerEvent::Finished {
            block,
            output: std::mem::take(&mut self.output),
        })
    }

    fn push_output(&mut self, t: &str) {
        self.output.push_str(t);
        if self.output.len() > EXCERPT_CAP {
            let mut cut = self.output.len() - EXCERPT_CAP;
            while !self.output.is_char_boundary(cut) {
                cut += 1;
            }
            self.output.drain(..cut);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scanner;

    fn run(bytes: &[u8]) -> (Vec<TrackerEvent>, BlockTracker) {
        let mut s = Scanner::new();
        let mut t = BlockTracker::new(TerminalId::new(), "zsh");
        let mut now = Timestamp::UNIX_EPOCH;
        let mut out = Vec::new();
        for ev in s.feed(bytes) {
            now += jiff::SignedDuration::from_millis(10);
            out.extend(t.on(ev, now));
        }
        (out, t)
    }

    const CYCLE: &[u8] = b"\x1b]6973;cwd=/w\x07\x1b]6973;git_branch=main\x07\x1b]133;A\x07$ \x1b]133;B\x07\
        \x1b]6973;cmd=bHMgLWw=\x07\x1b]133;C\x07a\r\nb\r\n\x1b]133;D;2\x07\x1b]133;A\x07$ \x1b]133;B\x07";

    #[test]
    fn full_cycle_produces_one_block() {
        let (evs, t) = run(CYCLE);
        assert_eq!(evs.len(), 2);
        let TrackerEvent::Started(start) = &evs[0] else {
            panic!("{evs:?}")
        };
        assert_eq!(start.command, "ls -l");
        assert_eq!(start.seq, 0);
        let TrackerEvent::Finished { block, output } = &evs[1] else {
            panic!("{evs:?}")
        };
        assert_eq!(block.exit_code, Some(2));
        assert_eq!(block.cwd.as_deref(), Some("/w"));
        assert_eq!(block.git_branch.as_deref(), Some("main"));
        assert!(block.finished_at.unwrap() > block.started_at);
        assert_eq!(output, "a\nb\n");
        assert!(!t.is_degraded());
    }

    #[test]
    fn seq_increments() {
        let mut bytes = CYCLE.to_vec();
        bytes.extend_from_slice(b"\x1b]133;C\x07\x1b]133;D;0\x07");
        let (evs, _) = run(&bytes);
        let seqs: Vec<u32> = evs
            .iter()
            .filter_map(|e| {
                if let TrackerEvent::Started(b) = e {
                    Some(b.seq)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(seqs, vec![0, 1]);
    }

    #[test]
    fn lost_d_is_closed_by_next_prompt() {
        let (evs, _) = run(b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07x\x1b]133;A\x07");
        assert!(matches!(&evs[1], TrackerEvent::Finished { block, .. } if block.exit_code.is_none()));
    }

    #[test]
    fn double_output_start_degrades_and_stops() {
        let (evs, t) = run(b"\x1b]133;A\x07\x1b]133;C\x07\x1b]133;C\x07\x1b]133;A\x07\x1b]133;C\x07");
        assert!(matches!(evs.last(), Some(TrackerEvent::Degraded(_))));
        assert_eq!(evs.len(), 2);
        assert!(t.is_degraded());
    }

    #[test]
    fn output_start_before_any_prompt_degrades() {
        let (evs, _) = run(b"\x1b]133;C\x07");
        assert!(matches!(evs[0], TrackerEvent::Degraded(_)));
    }

    #[test]
    fn repeated_marks_are_tolerated() {
        let (evs, t) = run(
            b"\x1b]133;A\x07\x1b]133;A\x07\x1b]133;B\x07\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;0\x07\x1b]133;D;0\x07",
        );
        assert_eq!(evs.len(), 2);
        assert!(!t.is_degraded());
    }

    #[test]
    fn alt_screen_output_not_captured() {
        let (evs, _) = run(b"\x1b]133;A\x07\x1b]133;C\x07before\n\x1b[?1049hTUI\x1b[?1049lafter\n\x1b]133;D;0\x07");
        let TrackerEvent::Finished { output, .. } = &evs[1] else {
            panic!()
        };
        assert_eq!(output, "before\nafter\n");
    }

    #[test]
    fn excerpt_keeps_tail_within_cap() {
        let mut t = BlockTracker::new(TerminalId::new(), "zsh");
        let now = Timestamp::UNIX_EPOCH;
        t.on(ScanEvent::Mark(Mark::PromptStart), now);
        t.on(ScanEvent::Mark(Mark::OutputStart), now);
        t.on(ScanEvent::Text("é".repeat(EXCERPT_CAP)), now);
        t.on(ScanEvent::Text("END".into()), now);
        let Some(TrackerEvent::Finished { output, .. }) = t.on(ScanEvent::Mark(Mark::Finished(Some(0))), now) else {
            panic!()
        };
        assert!(output.len() <= EXCERPT_CAP);
        assert!(output.ends_with("END"));
    }
}
