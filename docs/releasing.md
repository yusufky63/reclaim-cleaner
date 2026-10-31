# Releasing

## Cut a release

Releases come from a release PR that the **Release PR** workflow keeps up to date. Do not tag by hand.

1. Write user-visible changes under "Unreleased" in `CHANGELOG.md` as part of each change, as before.
2. After every push to `main`, while "Unreleased" has notes, the workflow opens or updates the PR `chore(release): x.y.z` from the `release/next` branch. It runs `node scripts/release.ts`, which:
   - picks the version from the Conventional Commits since the last tag: `feat` raises the minor version, anything else the patch version, and a breaking change (`feat!:`, `BREAKING CHANGE:`) the major version (still the minor version before 1.0);
   - sets it in `package.json`, `Cargo.toml`, `Cargo.lock` and the bug report form;
   - moves the "Unreleased" notes under `## [x.y.z] - YYYY-MM-DD` and adds the compare link.

   The PR is rebuilt from `main` on every push, so fix the notes on `main`, not in the PR. Other edits for the release, such as "Shipped in" in `ROADMAP.md`, also go to `main` first.
3. Merge the PR when you want to release. The workflow sees a version without a tag, tags it (`vX.Y.Z`) and calls the **Release** workflow.
4. The **Release** workflow checks that the tag matches `package.json`, runs the full check suite, builds the NSIS setup and the MSI packages, zips the portable build and writes `SHA256SUMS.txt`. It creates a **draft** release with the CHANGELOG section as notes.
5. Download the setup from the draft, install it on a clean Windows VM, smoke-test a scan and a clean, then publish the draft.

The workflow needs "Allow GitHub Actions to create and approve pull requests" under Settings > Actions > General. A pushed `v*` tag still starts the Release workflow, but only if it matches the version in `package.json`.

To preview the next version locally: `node scripts/release.ts plan`.

MSI versions must be numeric (`1.2.3`). Pre-release tags such as `1.2.0-beta.1` break the MSI build; ship betas as setup and portable only, or use a numeric fourth part (`1.2.0-1`).

## Build locally

```bash
pnpm app:build
pnpm portable
```

Everything ends up in `release/`.

## Code signing

Unsigned builds trigger SmartScreen. The free option for open-source projects is the [SignPath Foundation](https://signpath.org) program. It requires an OSI license, a release that builds in CI from this repository, MFA for maintainers, and the "Code signing policy" section in the README. Once accepted, add the SignPath GitHub action after the build step and sign the setup, the MSI files and the portable exe before computing checksums.

## winget

After the first published release:

```bash
komac new --id yusufky63.ReClaimCleaner --version X.Y.Z --urls <setup.exe url> <msi url>
```

Use `InstallerType: nullsoft` and `Scope: user` for the setup, `InstallerType: wix` and `Scope: machine` for the MSI. Later versions can be submitted with `komac update` or the `winget-releaser` action. See [packaging/winget](../packaging/winget/README.md).

## Scoop

`packaging/scoop/reclaim-cleaner.json` is a manifest for a personal bucket. Update `version`, `url` and `hash` (from `SHA256SUMS.txt`) and commit it to the bucket repository. `checkver` and `autoupdate` let `scoop` pick up later releases.
