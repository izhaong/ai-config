import { describe, expect, it } from "vitest";

import { parseMcpServerInput } from "./parseMcpServerInput";

describe("parseMcpServerInput", () => {
  it("parses single-key object", () => {
    const result = parseMcpServerInput(`{
      "ai-config": {
        "command": "ai-config",
        "args": ["serve"]
      }
    }`);
    expect(result.name).toBe("ai-config");
    expect(result.config).toEqual({
      command: "ai-config",
      args: ["serve"],
    });
  });

  it("unwraps mcpServers wrapper", () => {
    const result = parseMcpServerInput(`{
      "mcpServers": {
        "fetch": { "command": "uvx", "args": ["mcp-server-fetch"] }
      }
    }`);
    expect(result.name).toBe("fetch");
    expect(result.config.command).toBe("uvx");
  });

  it("accepts http url transport", () => {
    const result = parseMcpServerInput(`{
      "remote": { "url": "https://example.com/mcp" }
    }`);
    expect(result.name).toBe("remote");
    expect(result.config.url).toBe("https://example.com/mcp");
  });

  it("rejects empty input", () => {
    expect(() => parseMcpServerInput("   ")).toThrow(/不能为空/);
  });

  it("rejects invalid json", () => {
    expect(() => parseMcpServerInput("{ broken")).toThrow(/JSON 格式无效/);
  });

  it("rejects multiple servers", () => {
    expect(() =>
      parseMcpServerInput(`{
        "a": { "command": "echo" },
        "b": { "command": "echo" }
      }`),
    ).toThrow(/恰好包含一个/);
  });

  it("rejects config without command or url", () => {
    expect(() => parseMcpServerInput(`{ "bad": { "env": {} } }`)).toThrow(
      /command 或 url/,
    );
  });

  it("rejects non-object root", () => {
    expect(() => parseMcpServerInput(`["array"]`)).toThrow(/JSON object/);
  });
});
