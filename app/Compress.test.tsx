import { expect, test } from "nsh/test";
import { Compress } from "tinypng-mac/native";

test("native auto compresses png adaptively", async () => {
  const result = await Compress.auto({
    input: "tests/fixtures/icon.png",
    output: "/private/tmp/imgsquash-native-test.png",
    mode: "smart",
    preset: "standard",
    outputFormat: "keep",
  });
  expect(result.outputSize).toBeLessThan(result.inputSize);
  expect(result.lossy).toBe(false);
  expect(result.score).toBeGreaterThan(70);
  expect(result.score).toBe(100);
});

test("native auto quantizes icc png and keeps the profile", async () => {
  const result = await Compress.auto({
    input: "tests/fixtures/icon-icc.png",
    output: "/private/tmp/imgsquash-native-icc-test.png",
    mode: "smart",
    preset: "standard",
    outputFormat: "keep",
  });
  expect(result.lossy).toBe(false);
  expect(result.keptIcc).toBe(true);
  expect(result.outputSize).toBeLessThan(result.inputSize);
});

test("parallel auto calls give the same result as one alone", async () => {
  const args = {
    input: "tests/fixtures/icon.png",
    mode: "smart",
    preset: "standard",
    outputFormat: "keep",
  };
  const workers = Compress.parallelism();
  expect(workers).toBeGreaterThanOrEqual(1);
  expect(workers).toBeLessThanOrEqual(4);

  const started = Date.now();
  const parallel = await Promise.all(
    [1, 2, 3, 4].map(n => Compress.auto({ ...args, output: `/private/tmp/imgsquash-parallel-${n}.png` })),
  );
  const parallelMs = Date.now() - started;

  const singleStarted = Date.now();
  const single = await Compress.auto({ ...args, output: "/private/tmp/imgsquash-parallel-single.png" });
  const singleMs = Date.now() - singleStarted;

  // 每张都独立编码，并发不该改变任何一张的结果。
  for (const result of parallel) {
    expect(result.outputSize).toBe(single.outputSize);
    expect(result.score).toBe(single.score);
    expect(result.method).toBe(single.method);
    expect(result.ratio).toBe(single.ratio);
  }
  console.log(`parallel 4: ${parallelMs}ms, single: ${singleMs}ms, workers: ${workers}`);
});
