/** 从绝对路径取最后一级目录名，用作默认项目名 */
export function defaultProjectNameFromPath(rootPath: string): string {
  const normalized = rootPath.trim().replace(/[/\\]+$/, "");
  if (!normalized) return "";
  const last = normalized.split(/[/\\]/).pop();
  return last?.trim() ?? "";
}
