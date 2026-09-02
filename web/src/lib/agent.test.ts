import { expect, test } from "bun:test";
import { AGENT_CLIENTS, TOKEN_PLACEHOLDER, agentSnippet } from "./agent";

test("every client's snippet names the endpoint and carries the token as a bearer", () => {
  for (const { value } of AGENT_CLIENTS) {
    const snippet = agentSnippet(value, "panel.example.com", "ferr_abc");
    expect(snippet).toContain("https://panel.example.com/mcp");
    expect(snippet).toContain("Bearer ferr_abc");
  }
});

test("the JSON clients get valid JSON with one server called ferrum", () => {
  for (const client of ["claude-desktop", "cursor"] as const) {
    const parsed = JSON.parse(agentSnippet(client, "panel.example.com", TOKEN_PLACEHOLDER));
    expect(Object.keys(parsed.mcpServers)).toEqual(["ferrum"]);
  }
});

test("claude desktop keeps the header's space out of args", () => {
  const parsed = JSON.parse(agentSnippet("claude-desktop", "h", "t"));
  const args: string[] = parsed.mcpServers.ferrum.args;
  expect(args.at(-1)).toBe("Authorization:${AUTH_HEADER}");
  expect(parsed.mcpServers.ferrum.env.AUTH_HEADER).toBe("Bearer t");
});
