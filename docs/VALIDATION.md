# Validation report

Psychotalk is experimental and has not received an independent security audit. Passing importer, protocol, or emulator tests does not establish physical-device security or forensic erasure.

## Public distribution

This repository starts from a reviewed source snapshot. Earlier Git history, workflow logs, and unsanitized binaries remain in a private archive and were not imported into the public history. Public release provenance is recorded here after builds from the clean history complete.

- Desktop: 0.4.2 experimental, macOS ARM64/Intel and Linux x86_64.
- Android: 0.3.1 experimental, versionCode 6, ARM64. This rebuild remaps workstation paths, retains the existing debug signing certificate, and leaves the protocol and vault schema unchanged.
- Rust release builds use `scripts/private-build.py`. APK/archive contents are checked with `scripts/check-runtime-privacy.py` for the current workstation's source/home prefixes.
- Dependency author/license notices are retained. The privacy review concerns the project's private account/contact data, vaults, build paths, and credentials.

## Checks carried forward and rerun for public builds

| Area | Verification and scope |
|---|---|
| Rust | 32 core and 13 desktop cases, including authorization, real SQLCipher, TLS adversarial checks, migration, author deletion, and recipient acknowledgement. The OS-keyring test performs real operations only with `PSYCHOTALK_TEST_KEYRING=1`; a normal skipped-by-environment return is not keyring evidence. CI runs that test explicitly. Formatting and Clippy are required. |
| Desktop vault | Real Argon2id/AES-GCM/SQLCipher, two passwords, incorrect-password rejection, dependency on the OS key, reopen/cleanup, interrupted setup recovery, and refusal of unowned directories/symlinks. |
| Browser | Real backend, keyring, and Tor process. Contacts, local tagnames, isolated queued messages, fixed group creation, invitation preview/import, partial deletion state, lock/reopen, watchdog and cleanup. Chromium is used. |
| Delayed Tor identity | Only delivery of the real `view.own_code` to the UI is withheld, then released after completing the group form. The button must enable without editing or reopening, preserving name and contacts. The transport/database/keyring are not mocked. |
| Page resume | Reload before configuration, reload with open/locked vault, password required again, only a resume proof in sessionStorage, real popup/duplicated-tab takeover, rejected old tokens/proofs/commands/heartbeat/lock, delayed unlock reply, cleanup revocation, native restart, and denied sessionStorage. |
| Expiration | Native authorization tests use controlled times for the eight-hour limit, including wall-clock advance while monotonic time remains unchanged. No eight-hour real waiting or exhaustive physical suspension campaign was performed. |
| Local API | Exact Host/Origin, missing/foreign Origin rejection, document binding, role separation, one-use bootstrap/resume proofs, no permissive CORS, no-store/CSP, and body limits on endpoints that consume bodies. Generation is checked again before operations execute; old operations cannot affect a new grant. |
| Distribution | Native package extraction, relocated dependencies, bundled Tor discovery, version/source metadata, system keyrings, install/update/uninstall preserving data, and checksum-corruption rejection. Installer integration replaces only GitHub downloads with fixture downloads; live GitHub checks are recorded separately. |

The pre-publication desktop 0.4.2 passed the native three-platform matrix and a separate test of the same packaged files. An actual authenticated GitHub update from a disposable 0.4.1 SQLCipher/Keychain vault preserved identity, contact, and a queued message through reopen and page reload, then completed cleanup. An initial 30-second unlock attempt timed out; a longer retry passed. Its specific delay was not isolated. This is historical evidence, not a claim that a vault-format migration occurred.

## Earlier Tor/Android interoperability

Real Tor/mTLS tests used two desktop instances and Android 0.3.0 in an ARM64 API 30 emulator. They exchanged persisted messages in both directions, deleted each author's stream without affecting another author, formed a mixed fixed group, retained a pending recipient acknowledgement while Android was offline, and completed it after reconnecting. Desktop reopen retained the identity and OS-keyring cleanup was checked.

ADB carried public pairing codes, invitations, and test-progress markers only. Messages and acknowledgements crossed Tor. Instrumentation confirmed UI-redaction epochs as a test driver; this was not a complete touch-driven test between physical phones. It is historical protocol evidence, not a new physical-device proof for the privacy rebuild.

The prior Android regular suite reported 11 cases: eight effective cases passed; two conditional upgrade cases and the separate interoperability case were not exercised by the normal run. Conditional upgrade and Tor interoperability results are distinguished from that count. Public-rebuild results are recorded below after execution.

## Remaining limits

Physical Android devices on separate networks, attributed IPv4/IPv6 traffic capture and network switching, hardware-backed Keystore, 16 KiB Android runtime pages, other Linux distributions, Safari/Firefox, screen readers, and exhaustive failure/suspension testing remain unverified or incomplete. The emulator reported software Keystore protection. UI checks included 1440×1000, 400×850, CSS zoom at 200%, and RTL; CSS zoom is not a complete native-browser zoom/accessibility assessment.

An acknowledgement is an authenticated client's statement, not independent proof against a modified client. Offline peers retain their copies until a request is delivered/applied. Screenshots, photographs, exports, clipboard copies, backups, extensions, and residual memory can retain content. No absolute anonymity, global-correlation resistance, or forensic flash/RAM erasure is promised.

See [DESKTOP.md](DESKTOP.md) for reproducible desktop and interoperability commands and [SECURITY.md](SECURITY.md) for protocol/storage details.
