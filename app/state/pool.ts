/**
 * 固定并发跑一批任务：每项只跑一次，同时在跑的不会超过 `limit`。
 *
 * 取号 `next++` 是同步的，所以两个 worker 不会拿到同一项；`limit` 大于
 * 任务数时按任务数开 worker，空跑不会发生。
 */
export async function runPool<T>(
  items: readonly T[],
  limit: number,
  worker: (item: T, index: number) => Promise<void>,
  shouldStop: () => boolean = () => false,
): Promise<void> {
  let next = 0;
  const count = Math.max(1, Math.min(Math.floor(limit), items.length));
  const runners = Array.from({ length: count }, async () => {
    for (;;) {
      if (shouldStop()) return;
      const index = next++;
      if (index >= items.length) return;
      await worker(items[index], index);
    }
  });
  await Promise.all(runners);
}
