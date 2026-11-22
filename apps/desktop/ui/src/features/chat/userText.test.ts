import { expect, test } from "vitest";
import { splitUserText } from "./userText";

test("separates attached files from typed text", () => {
  const v = splitUserText(
    'explain @a.rs\n\n<attached_file path="a.rs">\nfn a() {}\n</attached_file>',
  );
  expect(v.text).toBe("explain @a.rs");
  expect(v.files).toEqual([{ path: "a.rs", body: "fn a() {}" }]);
  expect(splitUserText("plain").files).toEqual([]);
});
