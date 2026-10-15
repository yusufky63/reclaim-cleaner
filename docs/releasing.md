# Releasing

## Cut a release

1. Update `version` in `package.json` and `[workspace.package] version` in `Cargo.toml`. The app reads its version from `package.json`.
2. Move the "Unreleased" notes in `CHANGELOG.md` under a new `## [x.y.z] - YYYY-MM-DD` heading and update the links at the bottom.
3. Commit (`chore(release): x.y.z`), tag and push:

   ```bash
   git tag vX.Y.Z
   git push origin main --tags
   ```

4. The **Release** workflow runs the full check suite, builds the NSIS setup and the MSI packages, zips the portable build and writes `SHA256SUMS.txt`. It creates a **draft** release with the CHANGELOG section as notes.
5. Download the setup from the draft, install it on a clean Windows VM, smoke-test a scan and a clean, then publish the draft.

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
