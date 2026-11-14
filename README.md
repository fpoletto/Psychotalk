# Psychotalk — experimental messenger

Psychotalk is a text messenger for **Android 10+ on ARM64** and **macOS/Linux with a local browser interface**. It supports multiple contacts, individual conversations, and fixed groups of **3–5 participants**.

The Android interface uses Kotlin/Compose. The desktop interface is served by a local Rust process and opens in your browser. Both clients share the Rust protocol and SQLCipher storage, and bundle Tor. There is no central message server, messaging account, phone-number registration, analytics, Orbot dependency, or background delivery service.

**Experimental. No independent security audit has been performed.** Read the [security model](docs/SECURITY.md) and [validation report](docs/VALIDATION.md) before relying on it. Those detailed documents are currently in Portuguese; the application interface is also in Portuguese.

## Downloads

- [Desktop 0.4.2 experimental](https://github.com/fpoletto/Psychotalk/releases/tag/v0.4.2-experimental): macOS Apple Silicon, macOS Intel, and Linux x86_64 packages, installer, and SHA-256 checksums.
- [Desktop installer (`install.sh`)](https://github.com/fpoletto/Psychotalk/releases/download/v0.4.2-experimental/install.sh).
- [Android 0.3.1 experimental](https://github.com/fpoletto/Psychotalk/releases/tag/v0.3.1-experimental): main ARM64 APK and checksum.
- [Source repository](https://github.com/fpoletto/Psychotalk): source and documentation on `main`.

This public repository starts from a reviewed source snapshot; older history and artifacts were retained privately. See the [publication privacy review](docs/PUBLICATION.md).

Each installation has its own identity and local vault. Desktop does not mirror or import an Android identity automatically. The two clients can exchange messages and participate in the same fixed groups.

## Install on macOS or Linux

The current installer uses an authenticated **GitHub CLI (`gh`)**. Install `gh` and sign in first. This authentication is an installer requirement; it is separate from the messenger's local vault and passwords.

```sh
gh auth login
psychotalk_install_dir="$(mktemp -d)"
gh release download v0.4.2-experimental \
  --repo fpoletto/Psychotalk \
  --pattern install.sh \
  --dir "$psychotalk_install_dir" &&
sh "$psychotalk_install_dir/install.sh"

"$HOME/.local/bin/psychotalk"
```

The installer selects the native package, verifies its SHA-256 checksum, and installs it in your user directory. **Rust, Node.js, and Docker are not runtime requirements.** Tor and the required native libraries are bundled.

If `~/.local/bin` is on your `PATH`, you can use:

```sh
psychotalk          # Open the interface
psychotalk update   # Install the latest desktop release, including experimental releases
psychotalk stop     # Stop the local program
psychotalk uninstall # Remove the program while preserving the vault and keyring entry
```

Updates preserve the vault and stop the previous session. Removing the program also preserves the vault; use the cleanup password first if you intend to erase its contents. Installation and update requests to GitHub run outside the messenger's Tor transport.

Supported packages and validated environments:

| Package | Validation environment | Additional requirement |
|---|---|---|
| macOS ARM64 | macOS 15, Apple Silicon | Available macOS Keychain |
| macOS x86_64 | macOS 15, Intel | Available macOS Keychain |
| Linux x86_64 | Ubuntu 22.04 | Unlocked Secret Service, such as GNOME Keyring |

Other Linux distributions have not been validated. macOS packages use ad-hoc signing and are not Apple-notarized; the system may request Keychain authorization after an update. The program never falls back to storing its key in a plaintext file.

See [desktop instructions](docs/DESKTOP.md) for storage locations, advanced options, and developer commands.

## First use and page reloads

1. Open Psychotalk through the launcher. The first browser window receives a temporary authorization, which is removed from the address bar after loading. Do not share the launch URL.
2. Set two different passwords: a **normal password** to open the vault and a **local cleanup password** to destroy it. There is no password recovery.
3. Keep the app open and unlocked while sending or receiving. Hiding the tab or losing its heartbeat locks the session. Delivery resumes after both participants are open and unlocked again.

Starting with **desktop 0.4.2**, refreshing the page returns to the password screen without requiring another terminal command. It closes the vault before resuming the interface, so the normal password is still required.

Only a random, single-use **resume proof** is stored in `sessionStorage`. The command token remains in page memory. Passwords, keys, and messages are not stored there. Every resume rotates both credentials and invalidates the previous document. A duplicated tab that resumes takes control and locks the previous session.

The proof is valid only in the current native process and expires after eight hours of inactivity. Restarting the process, opening again through the launcher, or completing vault cleanup invalidates it. Browsers can restore session storage with tabs; server-side validation still applies. If storage is blocked or a resume exchange is interrupted, reopen through the launcher.

## Security advantages and their boundaries

| Advantage | Implemented protection | Boundary |
|---|---|---|
| No central message database | Messages travel directly between participants; histories and offline queues stay in encrypted local vaults. | Both peers must be online and unlocked for delivery. There is no central store-and-forward service. |
| Tor-only peer transport | Onion v3 services, bundled Tor, and no direct-IP fallback for participant traffic. | Tor use and traffic patterns can still be observed or correlated. Absolute anonymity is not promised. |
| Mutual peer authentication | TLS 1.3 with verified certificate fingerprints; users confirm public codes through a trusted channel. | Pairing security depends on actually checking fingerprints. A public code is stable and can correlate the same identity across conversations. |
| Encrypted local storage | SQLCipher protects history, contacts, identity, queues, and deletion state. Argon2id and AES-GCM protect password envelopes. An additional key is held by Android Keystore or the desktop OS keyring. | Desktop keyrings are not claimed to provide Android-equivalent hardware protection. A compromised OS, browser, or authorized client remains a risk. |
| Limited local browser access | Loopback-only HTTP, exact Host/Origin checks, document-bound command tokens, restrictive CSP, no permissive CORS, and no authentication cookies. Resume proofs cannot directly call command APIs. | Local HTTP authorization protects the interface; Tor/mTLS handles transport between participants. This is not remote browser hosting. |
| Automatic locking | Visibility handling and a native heartbeat watchdog revoke access. Resume requires a closed vault and a new document authorization. | Browser memory, extensions, screenshots, clipboard copies, and OS backups cannot be reliably erased or controlled. |
| Author-scoped deletion | Persistent deletion markers remove only the requesting author's messages. Each authenticated recipient confirms after applying the deletion and discarding the official UI snapshot. | An offline recipient retains its copy until reconnecting. A modified client can lie, and external copies cannot be removed. |
| Local cleanup | A separate password destroys the owned local vault and invalidates its OS-protected key. Interrupted cleanup is recovered before networking. | It does not erase other participants' copies or send remote deletion requests. It can destroy pending deletion requests. Forensic erasure of flash/RAM is not guaranteed. |
| Inspectable distribution inputs | Version locks, pinned dependency/source checksums, package checksums, and bundled dependency notices. | A checksum detects corruption; it is not an independent signature against a compromised repository or build system. |

These protections complement each other. Psychotalk does **not** implement Double Ratchet or MLS and has not received an independent security audit.

## Contacts and individual conversations

1. Install the main Android APK or launch the desktop client. On Android, **install updates over the existing app with the same signing key**. Uninstalling removes its vault and Keystore protection.
2. Unlock or configure the local vault. Supported Android upgrades from the older vault formats use transactional migrations and preserve identity, queues, history, and pending deletions. Downgrading a migrated vault is unsupported.
3. Choose **Adicionar contato** (add contact). Exchange public codes through a trusted channel and compare the complete certificate fingerprint. Both participants must confirm locally.
4. Set a **local tagname**, such as `@alice`. It is encrypted in your own vault, need not be unique, and is not sent to the other participant. Editing it does not change the identity or pairing. An empty name restores the abbreviated identity.
5. Select a conversation. Your messages align to the right in the Portuguese interface; received messages align to the left. The list shows connection/pending states without message-content previews.
6. Keep both apps open and unlocked. Offline messages remain in an encrypted queue without blocking other contacts. The transport supports up to four concurrent connections and rotates through larger lists. A delivery acknowledgement requires persistence on the receiving client.
7. **Apagar minhas mensagens desta conversa** (delete my messages in this conversation) removes your existing texts and queues the corresponding deletion request. Other authors, other conversations, and subsequent messages are preserved. The request stays pending until the recipient applies it and acknowledges.

On Android, tagname editing is under **Opções → Editar tagname**. Received links remain plain text. Duplicate contacts and conflicting onion/fingerprint pairings are rejected. There are no global usernames or external messaging accounts.

## Fixed groups

1. The creator pairs with 2–4 contacts and chooses **Novo grupo** (new group), making 3–5 participants including the creator. Name and membership remain fixed; a different roster requires a new group.
2. Share the invitation through a trusted channel. Each invited participant chooses **Entrar em grupo** (join group), reviews the complete roster, group fingerprint, and public codes, and explicitly accepts.
3. Keep all clients open and unlocked until everyone is ready. Sending is disabled until authenticated acceptance and synchronization complete. Since desktop 0.4.1, a valid creation form automatically enables when a delayed Tor identity becomes available, preserving its fields.
4. Each author sends directly to each recipient over Tor/mTLS. The UI shows per-recipient delivery and deletion counts. Offline recipients remain pending; other group members do not relay their messages.
5. Deleting your messages removes only your authored stream in that group. Completion requires a separate acknowledgement from every recipient. **Participantes e convite** (participants and invitation) shows individual pending/confirmed states.
6. Member tagnames remain local. Joining a group does not authorize individual conversations with its other members; those require separate pairing.

Invitations reveal the roster's public onion addresses and fingerprints, but contain no private keys. An identity outside the fixed roster cannot import the invitation. There is no join-later membership change, removal, leave operation, administrator rotation, or global ordering across authors. Ready status persists and does not mean every member is currently online.

## Android installation and development

The installed app is named **Psychotalk** and uses a speech-bubble icon with the letter P. Install the main `psychotalk-0.3.1-experimental-arm64.apk`. The separate `app-debug-androidTest.apk` is an instrumentation package with no launcher icon.

The Android build environment uses JDK 17, Rust 1.100.0, SDK 37.1, build-tools 36.0.0, and NDK 28.2.13676358. Tools and generated outputs remain outside Git. Adapt `scripts/env.sh` for your workstation and install the required SDK/NDK and Rust target.

```sh
sh scripts/build-android.sh
. scripts/env.sh
adb install -r dist/psychotalk-0.3.1-experimental-arm64.apk
cargo test --locked --workspace
```

Android 0.3.1 is a privacy rebuild: local build paths are remapped, with no protocol or vault-format change. It retains the existing debug signing certificate and increments the version code for in-place updates. The release build checks the APK contents for workstation source/home paths.

The Gradle Wrapper verifies its distribution checksum, and the build script verifies the Tor AAR checksum. No messaging backend needs provisioning. TLS tests require loopback sockets.

Instrumentation tests use dedicated directories and Keystore aliases, separate from the UI vault. Run them only on a device/emulator reserved for testing:

```sh
adb install -r android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
adb shell am instrument -w app.psychotalk.test/androidx.test.runner.AndroidJUnitRunner
```

Desktop build and browser-test instructions are in [DESKTOP.md](docs/DESKTOP.md); CI workflows are in [.github/workflows](.github/workflows).

## Validation and remaining limits

The [validation report](docs/VALIDATION.md) distinguishes native unit/integration tests, browser tests, emulator proofs, and unverified scenarios.

Desktop 0.4.2 was built and tested on macOS ARM64, macOS Intel, and Ubuntu 22.04. The distributed packages were checked with their bundled Tor and real OS keyrings. Reload/resume, duplicated tabs, stale commands, cleanup, native restart, denied browser storage, and native expiration checks are covered. Eight-hour expiration uses controlled native times in tests; no eight-hour physical suspension campaign was performed.

Earlier real Tor/mTLS tests exchanged persisted messages between desktop and Android 0.3.0 in an emulator, including a mixed group, partial deletion acknowledgements while Android was offline, and completion after reconnecting. This is historical protocol evidence, not a new physical-device test for 0.4.2.

Physical Android devices on distinct networks, exhaustive suspension/failure testing, hardware-backed Keystore, attributed IPv4/IPv6 traffic capture, other Linux distributions, Safari/Firefox, and screen-reader coverage remain unverified or incomplete. The Android APK is a debug build. Bootstrap and reconnection can take minutes.

Acknowledgements are authenticated client statements, not independent proof of destruction. Screenshots, photographs, copied/exported text, modified clients, backups, and residual memory can retain content. The system does not promise absolute anonymity, global traffic-correlation resistance, or unrecoverable physical deletion.
