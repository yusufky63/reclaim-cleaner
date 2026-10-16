# winget

winget manifests live in [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs), not here. Submit them after a release is published (not a draft).

Package identifier: `yusufky63.ReClaimCleaner`.

```bash
# First submission
komac new --id yusufky63.ReClaimCleaner --version 0.2.0 \
  --urls "https://github.com/yusufky63/reclaim-cleaner/releases/download/v0.2.0/ReClaim-Cleaner_0.2.0_x64-setup.exe"

# Later versions
komac update yusufky63.ReClaimCleaner --version X.Y.Z --urls <setup url> --submit
```

Installer facts komac asks for:

| Field | Setup (.exe) | MSI |
|---|---|---|
| InstallerType | `nullsoft` | `wix` |
| Scope | `user` | `machine` |
| Silent switch | `/S` (automatic for nullsoft) | `/quiet` (automatic for wix) |
| UpgradeCode | | `93A4F048-2E53-418D-A52F-361DA387369B` |

The release workflow names assets without spaces (`ReClaim-Cleaner_<version>_x64-setup.exe`), so the URLs are stable.
