import { expect, test } from "nsh/test";
import { runPool } from "./state/pool";

test("runPool runs every item once and never exceeds the limit", async () => {
  const items = Array.from({ length: 11 }, (_, index) => index);
  const seen: number[] = [];
  let running = 0;
  let peak = 0;
  await runPool(items, 3, async item => {
    running += 1;
    peak = Math.max(peak, running);
    seen.push(item);
    await new Promise(resolve => setTimeout(resolve, 5));
    running -= 1;
  });
  expect(seen.slice().sort((a, b) => a - b)).toEqual(items);
  expect(seen.length).toBe(items.length);
  expect(peak).toBe(3);
});

test("runPool opens no more workers than items", async () => {
  let running = 0;
  let peak = 0;
  await runPool([1, 2], 8, async () => {
    running += 1;
    peak = Math.max(peak, running);
    await new Promise(resolve => setTimeout(resolve, 5));
    running -= 1;
  });
  expect(peak).toBe(2);
});

test("runPool waits for every worker before resolving", async () => {
  const done: number[] = [];
  await runPool([1, 2, 3, 4, 5], 2, async item => {
    await new Promise(resolve => setTimeout(resolve, item % 3 === 0 ? 20 : 1));
    done.push(item);
  });
  expect(done.length).toBe(5);
});

test("stopping drains started workers without taking another item", async () => {
  let stop = false;
  const started: number[] = [];
  const finished: number[] = [];
  let release!: () => void;
  const blocked = new Promise<void>(resolve => { release = resolve; });
  const run = runPool([0, 1, 2, 3, 4], 2, async item => {
    started.push(item);
    await blocked;
    finished.push(item);
  }, () => stop);
  expect(started).toEqual([0, 1]);
  stop = true;
  release();
  await run;
  expect(finished.slice().sort()).toEqual([0, 1]);
  expect(started).toEqual([0, 1]);
  const resumed: number[] = [];
  await runPool([2, 3, 4], 2, async item => { resumed.push(item); });
  expect(resumed.slice().sort()).toEqual([2, 3, 4]);
});

test("a stopped pool does not start any work", async () => {
  const seen: number[] = [];
  await runPool([1, 2], 2, async item => { seen.push(item); }, () => true);
  expect(seen).toEqual([]);
});
