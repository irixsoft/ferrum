import { describe, expect, test } from "bun:test";
import { failureSentence } from "./github";

describe("failureSentence", () => {
  test("names each known reason in the panel's own words", () => {
    expect(failureSentence("no-code")).toBe("GitHub did not send a code.");
    expect(failureSentence("expired")).toBe("That connection attempt expired. Start again from Settings.");
    expect(failureSentence("refused")).toBe("GitHub refused the connection. Start again from Settings.");
  });

  test("never shows text from the URL", () => {
    for (const reason of [null, "", "Call +1 555 0100 to restore access", "constructor", "__proto__"]) {
      expect(failureSentence(reason)).toBe("GitHub refused the connection. Start again from Settings.");
    }
  });
});
