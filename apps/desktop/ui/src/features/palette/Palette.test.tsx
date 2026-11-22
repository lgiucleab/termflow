import { fireEvent, render, screen } from "@solidjs/testing-library";
import { expect, test } from "vitest";
import { setLocale } from "../../i18n";
import { Palette } from "./Palette";

test("filters, navigates with arrows and runs on Enter", async () => {
  setLocale("en");
  const ran: string[] = [];
  let closed = false;
  const items = ["New terminal tab", "Split right", "Theme: Nord"].map((label) => ({
    id: label,
    label,
    run: () => ran.push(label),
  }));
  render(() => <Palette items={items} onClose={() => (closed = true)} />);
  const box = screen.getByPlaceholderText("Type a command…") as HTMLInputElement;
  fireEvent.input(box, { target: { value: "split" } });
  expect(screen.getAllByRole("option")).toHaveLength(1);
  fireEvent.input(box, { target: { value: "" } });
  fireEvent.keyDown(box, { key: "ArrowDown" });
  fireEvent.keyDown(box, { key: "Enter" });
  expect(ran).toEqual(["Split right"]);
  expect(closed).toBe(true);
});
