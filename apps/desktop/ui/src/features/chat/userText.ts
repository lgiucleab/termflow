// User messages store attached files inline (`<attached_file path="…">…</attached_file>`) so the
// model sees them; the UI shows the typed text plus collapsed attachments.
export interface UserView {
  text: string;
  files: { path: string; body: string }[];
}

export function splitUserText(raw: string): UserView {
  const files: UserView["files"] = [];
  const text = raw
    .replace(
      /\n*<attached_file path="([^"]*)">\n([\s\S]*?)\n<\/attached_file>/g,
      (_m, path: string, body: string) => {
        files.push({ path, body });
        return "";
      },
    )
    .trimEnd();
  return { text, files };
}
