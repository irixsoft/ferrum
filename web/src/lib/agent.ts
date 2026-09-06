export type AgentClient = "claude-code" | "claude-desktop" | "cursor";

export const AGENT_CLIENTS: Array<{ value: AgentClient; label: string }> = [
  { value: "claude-code", label: "Claude Code" },
  { value: "claude-desktop", label: "Claude Desktop" },
  { value: "cursor", label: "Cursor" },
];

export const TOKEN_PLACEHOLDER = "<your token>";

/** Claude Desktop only speaks stdio, so it goes through mcp-remote; the header's space lives in the env var because Desktop splits args on spaces. */
export function agentSnippet(client: AgentClient, host: string, token: string): string {
  const url = `https://${host}/mcp`;
  const bearer = `Bearer ${token}`;
  switch (client) {
    case "claude-code":
      return `claude mcp add --transport http ferrum ${url} --header "Authorization: ${bearer}"`;
    case "claude-desktop":
      return JSON.stringify(
        {
          mcpServers: {
            ferrum: {
              command: "bunx",
              args: ["mcp-remote", url, "--header", "Authorization:${AUTH_HEADER}"],
              env: { AUTH_HEADER: bearer },
            },
          },
        },
        null,
        2,
      );
    case "cursor":
      return JSON.stringify(
        { mcpServers: { ferrum: { url, headers: { Authorization: bearer } } } },
        null,
        2,
      );
  }
}

export function agentSnippetFile(client: AgentClient): string {
  switch (client) {
    case "claude-code":
      return "Run in a terminal";
    case "claude-desktop":
      return "claude_desktop_config.json";
    case "cursor":
      return ".cursor/mcp.json";
  }
}
