import { render, screen, waitFor } from "@solidjs/testing-library";
import { expect, test } from "vitest";
import type { AppEvent, Bridge } from "../../bridge";
import { setLocale } from "../../i18n";
import { Home } from "./Home";

test("shows version from bridge and counts core events", async () => {
  setLocale("en");
  let emit: (e: AppEvent) => void = () => {};
  let started = false;
  const bridge: Bridge = {
    appInfo: async () => ({ name: "TermFlow", version: "9.9.9" }),
    startTicker: async () => {
      started = true;
    },
    onCoreEvent: async (h) => {
      emit = h;
      return () => {};
    },
  };

  render(() => <Home bridge={bridge} />);
  await waitFor(() => expect(screen.getByTestId("version").textContent).toBe("Version 9.9.9"));
  await waitFor(() => expect(started).toBe(true));
  emit({ type: "tick", n: 7 });
  await waitFor(() =>
    expect(screen.getByTestId("ticks").textContent).toBe("Events received from core: 7"),
  );
});
