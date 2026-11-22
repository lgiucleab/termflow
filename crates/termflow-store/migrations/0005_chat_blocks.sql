-- 0005: link command blocks to the chat whose shell ran them (unified timeline, decision 0002).
ALTER TABLE blocks ADD COLUMN chat_session_id TEXT;
CREATE INDEX blocks_chat ON blocks(chat_session_id, started_at);
