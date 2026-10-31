import { describe, expect, it } from "vitest";
import { bumpFor, cutChangelog, nextVersion, notesFor, setCargoVersion, setIssueVersion, setLockVersions, setPackageVersion } from "./release";

const CHANGELOG = `# Changelog

Intro.

## [Unreleased]

### Added

- New page.

## [0.2.0] - 2026-10-16

### Added

- Old page.

[Unreleased]: https://github.com/o/r/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/o/r/compare/v0.1.0...v0.2.0
`;

describe("version bump", () => {
  it("reads the bump from Conventional Commits", () => {
    expect(bumpFor(["fix(ui): a", "chore: b"])).toBe("patch");
    expect(bumpFor(["fix(ui): a", "feat(engine): b"])).toBe("minor");
    expect(bumpFor(["feat!: drop x"])).toBe("major");
    expect(bumpFor(["fix: a\n\nBREAKING CHANGE: settings moved"])).toBe("major");
    expect(bumpFor(["docs: mention feat: in text"])).toBe("patch");
  });

  it("keeps breaking changes minor before 1.0", () => {
    expect(nextVersion("0.2.0", "patch")).toBe("0.2.1");
    expect(nextVersion("0.2.5", "minor")).toBe("0.3.0");
    expect(nextVersion("0.2.5", "major")).toBe("0.3.0");
    expect(nextVersion("1.4.2", "major")).toBe("2.0.0");
    expect(() => nextVersion("1.2.0-beta.1", "patch")).toThrow();
  });
});

describe("changelog", () => {
  it("moves the Unreleased notes under the new version", () => {
    const out = cutChangelog(CHANGELOG, "0.3.0", "2026-11-01");
    expect(notesFor(out, "Unreleased")).toBe("");
    expect(notesFor(out, "0.3.0")).toBe("### Added\n\n- New page.");
    expect(notesFor(out, "0.2.0")).toBe("### Added\n\n- Old page.");
    expect(out).toContain("## [Unreleased]\n\n## [0.3.0] - 2026-11-01\n\n### Added");
    expect(out).toContain("[Unreleased]: https://github.com/o/r/compare/v0.3.0...HEAD\n[0.3.0]: https://github.com/o/r/compare/v0.2.0...v0.3.0\n[0.2.0]:");
  });

  it("refuses an empty Unreleased section", () => {
    const empty = cutChangelog(CHANGELOG, "0.3.0", "2026-11-01");
    expect(() => cutChangelog(empty, "0.4.0", "2026-11-02")).toThrow(/no Unreleased notes/);
  });

  it("reads the last section up to the link list", () => {
    const only = "## [Unreleased]\n\n## [0.1.0] - 2026-10-16\n\n- First.\n\n[Unreleased]: x\n";
    expect(notesFor(only, "0.1.0")).toBe("- First.");
  });
});

describe("version fields", () => {
  it("sets the app, workspace, lock file and issue form versions", () => {
    expect(setPackageVersion('{\n  "name": "x",\n  "version": "0.2.0",\n  "deps": { "a": "1.0.0" }\n}\n', "0.3.0")).toContain('"version": "0.3.0"');
    const cargo = '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.2.0"\nedition = "2024"\n';
    expect(setCargoVersion(cargo, "0.3.0")).toContain('[workspace.package]\nversion = "0.3.0"');
    const lock = '[[package]]\nname = "rayon"\nversion = "1.12.0"\n\n[[package]]\nname = "reclaim-cleaner"\nversion = "0.2.0"\n\n[[package]]\nname = "reclaim-core"\nversion = "0.2.0"\n';
    const bumped = setLockVersions(lock, "0.3.0");
    expect(bumped).toContain('name = "reclaim-cleaner"\nversion = "0.3.0"');
    expect(bumped).toContain('name = "reclaim-core"\nversion = "0.3.0"');
    expect(bumped).toContain('name = "rayon"\nversion = "1.12.0"');
    const form = "      label: ReClaim version\n      description: Settings > About.\n      placeholder: 0.2.0\n";
    expect(setIssueVersion(form, "0.3.0")).toContain("placeholder: 0.3.0");
    expect(() => setCargoVersion("[package]\nversion = \"1\"\n", "0.3.0")).toThrow();
  });
});
