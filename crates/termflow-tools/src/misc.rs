//! todo and attach_block.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use termflow_types::BlockId;

use crate::{CallFacts, Tool, ToolCtx, ToolOutput, arg_str};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    pub content: String,
    pub status: TodoStatus,
}

pub struct TodoTool;

#[async_trait::async_trait]
impl Tool for TodoTool {
    fn name(&self) -> &'static str {
        "todo"
    }
    fn description(&self) -> &'static str {
        "Replace the session's todo list (shown to the user). Use it to plan multi-step work and mark progress. Send the full list each time."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"todos":{"type":"array","items":{"type":"object","properties":{
            "content":{"type":"string"},
            "status":{"type":"string","enum":["pending","in_progress","completed"]}
        },"required":["content","status"],"additionalProperties":false}}},"required":["todos"],"additionalProperties":false})
    }
    fn facts(&self, args: &Value, _ctx: &ToolCtx) -> CallFacts {
        let n = args.get("todos").and_then(Value::as_array).map_or(0, Vec::len);
        CallFacts {
            summary: format!("todo ({n} items)"),
            ..CallFacts::default()
        }
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput {
        let todos: Vec<Todo> = match args.get("todos").cloned().map(serde_json::from_value) {
            Some(Ok(t)) => t,
            _ => return ToolOutput::err("`todos` must be a list of {content, status}"),
        };
        let mut out = String::new();
        for t in &todos {
            let mark = match t.status {
                TodoStatus::Pending => "[ ]",
                TodoStatus::InProgress => "[~]",
                TodoStatus::Completed => "[x]",
            };
            let _ = writeln!(out, "{mark} {}", t.content);
        }
        if let Ok(mut g) = ctx.todos.lock() {
            *g = todos;
        }
        ToolOutput::ok(if out.is_empty() {
            "(todo list cleared)".into()
        } else {
            out
        })
    }
}

pub struct AttachBlock;

#[async_trait::async_trait]
impl Tool for AttachBlock {
    fn name(&self) -> &'static str {
        "attach_block"
    }
    fn description(&self) -> &'static str {
        "Get a command and its output from the user's terminal. Without block_id, returns the most recent finished command."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"block_id":{"type":"string"}},"additionalProperties":false})
    }
    fn facts(&self, _args: &Value, _ctx: &ToolCtx) -> CallFacts {
        CallFacts {
            summary: "read terminal block".into(),
            ..CallFacts::default()
        }
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(lookup) = &ctx.blocks else {
            return ToolOutput::err("no terminal is available");
        };
        let id = arg_str(&args, "block_id").and_then(|s| s.parse().ok()).map(BlockId);
        match lookup(id) {
            Some((b, output)) => ToolOutput::ok(format!(
                "$ {}\n[exit {} · cwd {}]\n{}",
                b.command,
                b.exit_code.map_or_else(|| "?".into(), |c| c.to_string()),
                b.cwd.unwrap_or_default(),
                output
            )),
            None => ToolOutput::err("no such terminal block"),
        }
    }
}
