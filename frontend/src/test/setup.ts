import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";

afterEach(() => cleanup());

class MemScroll {
  scrollHeight = 0;
  scrollTo() {}
}
Object.defineProperty(HTMLElement.prototype, "scrollTo", {
  value() {},
  writable: true,
});
void MemScroll;
