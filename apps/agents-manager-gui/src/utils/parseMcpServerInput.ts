export interface ParsedMcpServer {
  name: string;
  config: Record<string, unknown>;
}

const EXAMPLE = `{
  "agents-manager": {
    "command": "agents-manager",
    "args": ["serve"]
  }
}`;

export const MCP_SERVER_INPUT_PLACEHOLDER = EXAMPLE;

function parseSingleServerObject(
  obj: Record<string, unknown>,
): ParsedMcpServer {
  const keys = Object.keys(obj);
  if (keys.length !== 1) {
    throw new Error("须恰好包含一个 MCP server（一个 key）");
  }
  const name = keys[0]!;
  const config = obj[name];
  if (!config || typeof config !== "object" || Array.isArray(config)) {
    throw new Error(`「${name}」的值须为 JSON object`);
  }
  const cfg = config as Record<string, unknown>;
  if (!cfg.command && !cfg.url) {
    throw new Error("server 须包含 command 或 url 字段");
  }
  return { name, config: cfg };
}

/** 解析用户粘贴的单个 MCP server JSON（支持 mcpServers 包装）。 */
export function parseMcpServerInput(raw: string): ParsedMcpServer {
  const trimmed = raw.trim();
  if (!trimmed) {
    throw new Error("JSON 不能为空");
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch (e) {
    throw new Error(`JSON 格式无效: ${String(e)}`);
  }

  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("须为 JSON object");
  }

  const obj = parsed as Record<string, unknown>;

  if (
    "mcpServers" in obj &&
    obj.mcpServers &&
    typeof obj.mcpServers === "object" &&
    !Array.isArray(obj.mcpServers)
  ) {
    return parseSingleServerObject(obj.mcpServers as Record<string, unknown>);
  }

  return parseSingleServerObject(obj);
}
