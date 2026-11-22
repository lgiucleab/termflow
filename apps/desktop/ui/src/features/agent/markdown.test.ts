import { expect, test } from "vitest";
import { renderMarkdown } from "./markdown";

test("renders markdown and strips scripts", () => {
  const html = renderMarkdown(
    "**hi** <script>alert(1)</script> <img src=x onerror=alert(1)>\n\n```rust\nfn main(){}\n```",
  );
  expect(html).toContain("<strong>hi</strong>");
  expect(html).toContain("<code");
  expect(html).not.toContain("<script");
  expect(html).not.toContain("onerror");
  expect(html).not.toContain("<img");
});
