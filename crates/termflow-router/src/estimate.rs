//! Token estimates for context fitting and proactive rate-limit switching.

use termflow_providers::ToolSpec;
use termflow_types::{Part, ToolResultContent, Turn};

/// Rough prompt size in tokens: JSON length / 4 over parts, system prompt and tool schemas.
// shortcut: no tokenizer; JSON overhead keeps it on the high side. Upgrade to provider
// count-tokens endpoints if overflows slip through in practice.
pub fn estimate(transcript: &[Turn], system: &str, tools: &[ToolSpec]) -> u32 {
    let parts: usize = transcript
        .iter()
        .map(|t| serde_json::to_string(&t.parts).map_or(0, |s| s.len()))
        .sum();
    let tools: usize = tools
        .iter()
        .map(|t| t.name.len() + t.description.len() + t.parameters.to_string().len())
        .sum();
    u32::try_from((parts + system.len() + tools) / 4).unwrap_or(u32::MAX)
}

const REMOVED: &str = "[output removed to fit the context window]";

/// Shrink a request copy to about `budget` tokens by blanking the oldest tool-result bodies.
/// The last `keep_last` turns, every tool call and every call/result pairing stay intact,
/// so the history remains valid for every wire format. Returns whether it now fits.
/// Only the rendered copy changes; the stored transcript is never touched.
pub fn trim_tool_results(
    transcript: &mut [Turn],
    system: &str,
    tools: &[ToolSpec],
    budget: u32,
    keep_last: usize,
) -> bool {
    let cut = transcript.len().saturating_sub(keep_last);
    for i in 0..cut {
        if estimate(transcript, system, tools) <= budget {
            return true;
        }
        for p in &mut transcript[i].parts {
            if let Part::ToolResult { content, .. } = p {
                *content = vec![ToolResultContent::Text { text: REMOVED.into() }];
            }
        }
    }
    estimate(transcript, system, tools) <= budget
}

#[cfg(test)]
mod tests {
    use super::*;
    use termflow_types::{CallId, Role, SessionId, TurnStatus};

    fn history(n: usize) -> Vec<Turn> {
        let s = SessionId::new();
        (0..n)
            .flat_map(|_| {
                let id = CallId::new();
                [
                    Turn::new(
                        s,
                        Role::Assistant,
                        vec![Part::ToolCall {
                            call_id: id,
                            name: "read".into(),
                            args: serde_json::json!({"path": "a"}),
                        }],
                        TurnStatus::Committed,
                    ),
                    Turn::new(
                        s,
                        Role::Tool,
                        vec![Part::ToolResult {
                            call_id: id,
                            content: vec![ToolResultContent::Text { text: "x".repeat(4000) }],
                            is_error: false,
                        }],
                        TurnStatus::Committed,
                    ),
                ]
            })
            .collect()
    }

    #[test]
    fn estimate_grows_with_content() {
        assert!(estimate(&history(1), "", &[]) > 1000);
        assert!(estimate(&history(4), "", &[]) > 3 * estimate(&history(1), "", &[]));
    }

    #[test]
    fn trim_blanks_old_results_but_keeps_pairs_and_recent_turns() {
        let mut h = history(5);
        let before = estimate(&h, "", &[]);
        assert!(trim_tool_results(&mut h, "", &[], before / 3, 2));
        assert_eq!(h.len(), 10);
        let Part::ToolResult { content, .. } = &h[1].parts[0] else {
            panic!()
        };
        assert_eq!(content, &vec![ToolResultContent::Text { text: REMOVED.into() }]);
        let Part::ToolResult { content, .. } = &h[9].parts[0] else {
            panic!()
        };
        assert_eq!(content[0], ToolResultContent::Text { text: "x".repeat(4000) });
        assert!(matches!(h[8].parts[0], Part::ToolCall { .. }));
        assert!(!trim_tool_results(&mut history(5), "", &[], 10, 2));
    }
}
