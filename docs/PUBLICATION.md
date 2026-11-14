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
