# Publication privacy review

The public repository begins with a clean source snapshot and a project-only author identity. The original repository, its Git history, workflow records, and older release files are retained privately. Their Git objects were not pushed into the public repository.

The reviewed source excludes local tools, transfer utilities, credentials, signing keystores, vaults, runtime databases, generated binaries, and test artifacts. Release assets are supplied separately after review. Public ownership necessarily identifies the GitHub account hosting this project; third-party authors' license notices are preserved.

Rust release builds remap workstation source/home paths to generic build paths. Android 0.3.1 is a new versionCode so existing installations can update without uninstalling. The signing certificate is retained; its private signing key is not distributed. Desktop packages are rebuilt from this source history and identify their source commit in BUILD.json.

Review before release:

1. Inspect tracked source and Git author/committer metadata, including a local secret scan.
2. Extract and inspect APK/tar contents for private path/account/contact markers, rather than scanning compressed bytes only.
3. Check native package checksums and source metadata.
4. Run relevant native, OS-keyring, browser, Android and upgrade checks; report the actual scope in VALIDATION.md.
5. Keep unrelated repositories' visibility unchanged.

This is a scoped privacy review, not a guarantee of perfect PII/secret detection or a security audit. Checksum verification detects file corruption and does not independently authenticate a compromised repository/build system.

## Verified publication state

The original repository retains its original GitHub identity and is private/archived. The replacement is a standalone public repository with a new root commit, followed only by newly reviewed commits. A clone with credential helpers and global Git configuration disabled contained no original private-history commit. Anonymous GitHub requests could access the new repository but could not access the private archive or the old commit through the new repository.

Known private account/email markers were absent from the reviewed source and public commit metadata. The Android release was checked entry-by-entry and as raw/signature bytes; its certificate subject is generic. A local Gitleaks scan found no credential findings in the clean source history. These findings are scoped checks, not a promise of perfect detection.

The comparison of all other existing repository visibility/archive states matched the pre-migration inventory. Only the approved project reorganization changed those states.

Public Android and desktop release assets were verified against their published SHA-256 values. Both tags point directly to clean commits and introduce no separate tagger email. The public installer download/install/update/uninstall check used a disposable directory; no personal vault was accessed. Detailed build and test links are in [VALIDATION.md](VALIDATION.md).
