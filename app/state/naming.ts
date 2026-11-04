/**
 * 输出文件名的候选：`pic.png` → `pic-2.png` → `pic-3.png`。
 *
 * 点号在开头（`.hidden`）或没有扩展名时整名当主干，不切出空扩展名。
 */
export function candidateName(name: string, suffix: number) {
  if (suffix <= 1) return name;
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return `${name}-${suffix}`;
  return `${name.slice(0, dot)}-${suffix}${name.slice(dot)}`;
}

/**
 * 占位用的键：APFS 默认不分大小写，`pic.png` 和 `Pic.png` 是同一个文件，
 * 所以按小写比较，宁可多跳一个后缀，也不要两个任务写同一个路径。
 */
export function reservationKey(path: string) {
  return path.toLowerCase();
}
