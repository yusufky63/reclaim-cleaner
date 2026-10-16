import { describe, expect, it } from "vitest";
import { baseName, daysSince, exeFromCommand, extOf, formatAgo, formatBytes, formatPercent, parentPath, shortPath, splitBytes } from "./format";

describe("formatBytes", () => {
  it("uses binary units with Explorer labels", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(13.4 * 1024 ** 3)).toBe("13.4 GB");
    expect(formatBytes(250 * 1024 ** 3)).toBe("250 GB");
  });

  it("follows the locale's decimal separator", () => {
    expect(formatBytes(13.4 * 1024 ** 3, "tr")).toBe("13,4 GB");
  });

  it("splits value and unit", () => {
    expect(splitBytes(41.7 * 1024 ** 3)).toEqual({ value: "41.7", unit: "GB" });
  });
});

describe("paths", () => {
  it("extracts names and parents", () => {
    expect(baseName("C:\\Users\\a\\Desktop\\")).toBe("Desktop");
    expect(parentPath("C:\\Users\\a\\file.txt")).toBe("C:\\Users\\a");
    expect(extOf("Movie.MKV")).toBe("mkv");
    expect(extOf(".gitignore")).toBe("");
  });

  it("extracts the program from a command line", () => {
    expect(exeFromCommand('"C:\\Program Files\\App\\app.exe" --minimized')).toBe("C:\\Program Files\\App\\app.exe");
    expect(exeFromCommand("C:\\Tools\\run.exe /background")).toBe("C:\\Tools\\run.exe");
    expect(exeFromCommand("%ProgramFiles%\\x\\y.exe")).toBe("%ProgramFiles%\\x\\y.exe");
  });

  it("shortens the profile prefix case-insensitively", () => {
    expect(shortPath("C:\\Users\\Alex\\.npm", "c:\\users\\alex")).toBe("~\\.npm");
    expect(shortPath("D:\\data", "C:\\Users\\Alex")).toBe("D:\\data");
    expect(shortPath("C:\\Users\\Alex", "C:\\Users\\Alex")).toBe("C:\\Users\\Alex");
    expect(shortPath("C:\\Users\\Alex2\\x", "C:\\Users\\Alex")).toBe("C:\\Users\\Alex2\\x");
  });
});

describe("time", () => {
  const now = Date.UTC(2026, 9, 16);
  it("counts days and formats relative time", () => {
    expect(daysSince(now / 1000 - 3 * 86400, now)).toBe(3);
    expect(daysSince(0, now)).toBe(Infinity);
    expect(formatAgo(now / 1000 - 3 * 86400, "en", now)).toBe("3 days ago");
    expect(formatAgo(now / 1000 - 3 * 86400, "tr", now)).toBe("3 gün önce");
  });

  it("formats percentages", () => {
    expect(formatPercent(1, 4)).toBe("25%");
    expect(formatPercent(0, 0)).toBe("0%");
  });
});
