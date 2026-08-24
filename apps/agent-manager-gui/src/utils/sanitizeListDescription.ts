/**
 * 列表/抽屉展示用：去掉 YAML frontmatter 泄漏的 block scalar 符号
 * （如 `description: >-`、单独的 `>-` / `|+` 等）。
 */
const DESCRIPTION_PREFIX = /^description:\s*(?:(?:>[-+]?|\|[-+]?)\s*)?(.*)$/is;

const BLOCK_SCALAR_ONLY = /^(?:>[-+]?|\|[-+]?)$/;

const LEADING_BLOCK_SCALAR = /^(?:>[-+]?|\|[-+]?)\s*/;

const TRAILING_BLOCK_SCALAR = /\s*(?:>[-+]?|\|[-+]?)\s*$/;

export function sanitizeListDescription(raw: string): string {
  if (!raw) return "";

  let s = raw.trim();
  if (BLOCK_SCALAR_ONLY.test(s)) {
    return "";
  }

  const descMatch = s.match(DESCRIPTION_PREFIX);
  if (descMatch) {
    s = descMatch[1].trim();
  }

  s = s.replace(LEADING_BLOCK_SCALAR, "");
  s = s.replace(TRAILING_BLOCK_SCALAR, "");
  s = s.replace(/^["']|["']$/g, "");
  return s.replace(/\s+/g, " ").trim();
}
