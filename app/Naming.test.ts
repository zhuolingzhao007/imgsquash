import { expect, test } from "nsh/test";
import { candidateName, reservationKey } from "./state/naming";

test("candidate names walk pic.png → pic-2.png → pic-3.png", () => {
  expect(candidateName("pic.png", 1)).toBe("pic.png");
  expect(candidateName("pic.png", 2)).toBe("pic-2.png");
  expect(candidateName("pic.png", 10)).toBe("pic-10.png");
  expect(candidateName("a.b.jpeg", 2)).toBe("a.b-2.jpeg");
});

test("candidate names keep names without a usable extension whole", () => {
  expect(candidateName("README", 3)).toBe("README-3");
  expect(candidateName(".hidden", 2)).toBe(".hidden-2");
});

test("reservation keys ignore case", () => {
  expect(reservationKey("/tmp/Pic.PNG")).toBe("/tmp/pic.png");
  expect(reservationKey("/tmp/pic.png")).toBe(reservationKey("/tmp/Pic.png"));
});
