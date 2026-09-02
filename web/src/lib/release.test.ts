import { describe, expect, test } from "bun:test";
import { summary } from "./release";

describe("summary", () => {
  test("skips headings and joins the first paragraph", () => {
    const notes = "## What's Changed\n\n* Faster deploys\n* Smaller binary\n\n## Full Changelog\n...";
    expect(summary(notes)).toBe("Faster deploys Smaller binary");
  });

  test("is empty when the notes are", () => {
    expect(summary("")).toBe("");
    expect(summary("# Only a title")).toBe("");
  });
});
