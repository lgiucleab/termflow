-- 0004: recent projects; tool-call execution guard.
ALTER TABLE workspaces ADD COLUMN last_opened TEXT;
CREATE INDEX sessions_workspace ON sessions(workspace_id, updated_at);
CREATE INDEX tool_calls_session ON tool_calls(session_id);
-- Full tool output (the model may only see a truncated copy).
ALTER TABLE tool_calls ADD COLUMN result_full TEXT;
