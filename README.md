<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="The Tunnel logo">
</p>

<h1 align="center">The Tunnel</h1>

<p align="center">
  Move and sync files between your computers over the same Wi-Fi or phone hotspot,<br>
  directly, encrypted, and without using any internet or mobile data.
</p>

---

## Contents

- [What it does](#what-it-does)
- [Download and install](#download-and-install)
  - [Windows](#windows)
  - [macOS](#macos)
- [First steps](#first-steps)
- [Using The Tunnel](#using-the-tunnel)
- [Build from source](#build-from-source)
  - [Windows prerequisites](#windows-prerequisites)
  - [macOS prerequisites](#macos-prerequisites)
  - [Get the code and run it](#get-the-code-and-run-it)
  - [Build an installer](#build-an-installer)
- [Publishing a release on GitHub](#publishing-a-release-on-github)
- [Troubleshooting](#troubleshooting)
- [Uninstalling](#uninstalling)
- [How it works](#how-it-works)
- [Security and privacy](#security-and-privacy)
- [Project layout](#project-layout)
- [Development notes](#development-notes)
- [Status and roadmap](#status-and-roadmap)

---

## What it does

| | |
| --- | --- |
| **Send** | Drag files or folders onto the window, use *Choose files*, or right-click a file and choose **Send with The Tunnel**. |
| **Sync** | A sync folder (default `Downloads/The Tunnel`) is kept identical on your paired devices. New and changed files are copied across automatically. |
| **Deletions are never automatic** | If a file is deleted on one device, the other device asks you to **Delete** it (to the Trash / Recycle Bin) or **Keep** it (which restores it on the first device). |
| **Conflicts** | If the same file is edited on both devices, both versions are kept: the newer one keeps the name and the other is saved as `name (conflict from <device> <time>).ext`. |
| **Browse and copy** | Browse the other computer (home folder and every drive), read-only, and copy anything to your Downloads. |
| **Resumable** | Interrupted transfers can be retried from where they stopped. |
| **Stays out of the way** | Runs in the menu bar or system tray, opens at login, and shows notifications. |
| **Local only** | Devices talk directly over the local network. Nothing is sent to the internet. |

Supported systems:

| System | Status |
| --- | --- |
| Windows 10 and 11 (64-bit) | Supported and tested |
| macOS 11 Big Sur or later (Apple Silicon and Intel) | Supported and tested |

Linux isn't built or supported for now.

---

## Download and install

Download the installer for your system from the **[Releases](../../releases)** page of this repository.

| System | File to download |
| --- | --- |
| Windows | `The Tunnel_<version>_x64-setup.exe` (or the `.msi`) |
| Mac with Apple Silicon (M1, M2, M3, M4…) | `The Tunnel_<version>_aarch64.dmg` |
| Mac with an Intel processor | `The Tunnel_<version>_x64.dmg` |

> **Not sure which Mac you have?** Open Apple menu → **About This Mac**. "Chip: Apple M…" means Apple Silicon; "Processor: Intel…" means Intel.

> The app isn't code-signed yet, so Windows and macOS warn the first time you open it. The steps below show how to get past that safely.

### Windows

1. Run `The Tunnel_<version>_x64-setup.exe`.
2. If **Windows protected your PC** (SmartScreen) appears, click **More info** → **Run anyway**.
3. Follow the installer. Near the end, Windows asks for administrator permission **once**. This lets The Tunnel through Windows Firewall on every network type. Phone hotspots usually count as "Public" networks, which Windows blocks by default. Click **Yes**.
4. The Tunnel opens. Installing a newer version over an older one keeps your settings and pairings.

### macOS

1. Open the `.dmg` and drag **The Tunnel** into **Applications**.
2. Open The Tunnel from Applications. Because it isn't signed, macOS blocks it the first time:
   - **macOS 15 Sequoia and later:** open **System Settings → Privacy & Security**, scroll down to the message about The Tunnel, and click **Open Anyway**.
   - **macOS 14 and earlier:** right-click (or Control-click) The Tunnel in Applications and choose **Open**, then **Open** again.
   - If macOS says the app **"is damaged and can't be opened"**, that's the download quarantine flag, not real damage. Run this in Terminal, then open it again:
     ```sh
     xattr -dr com.apple.quarantine "/Applications/The Tunnel.app"
     ```
3. When asked, **allow** The Tunnel to:
   - **find devices on your local network** (required),
   - **send notifications** (recommended),
   - access **Desktop, Documents, Downloads** or external drives when your other computer browses them (only needed if you use *Browse*).

---

## First steps

1. **Install The Tunnel on both computers**, and connect both to the **same** Wi-Fi or phone hotspot. No internet connection is needed.
2. **Open it on both.** Enter your name and a name for each device (for example "Arun's MacBook" and "Arun's PC").
3. Within a few seconds each computer shows **"… is on this network. Connect to it?"** Click **Pair**.
4. **Both screens show a 6-digit code.** Check that the codes match, then click **Codes match** on both. Pairing only happens once; after that the devices recognise each other automatically.

> **Devices don't see each other?** Click **Connect by address** on one computer and type the address shown in the other computer's **Settings** (at the bottom). See also [Troubleshooting](#troubleshooting).

---

## Using The Tunnel

### Send files

- **Drag and drop** files or folders onto the window while a connected device is selected, or click **Choose files** / **Choose folder** on the **Send** tab.
- **Right-click menu:**
  - **macOS:** right-click in Finder → **Quick Actions** → **Send with The Tunnel**. On older macOS it's under **Services**.
  - **Windows 10:** right-click → **Send with The Tunnel**.
  - **Windows 11:** right-click → **Show more options** → **Send with The Tunnel**.
  - If exactly one paired device is connected, the files go straight to it; otherwise The Tunnel asks which device to use.
- **Where files arrive:** in **Downloads** on the other computer (change this in Settings). Nothing is ever overwritten: a second `Photos` arrives as `Photos (1)`.

### Sync a folder

- Everything in your **sync folder** (default `Downloads/The Tunnel`) is kept identical on your paired devices, within a few seconds while both are on the same network, and catching up when they reconnect.
- Change the folder from the **Sync** tab (**Change…**) or in Settings. Whole drives, your entire home folder, and folders containing The Tunnel's own data can't be chosen.
- **Deleted on the other device:** a red banner and a badge on the **Sync** tab list these files. Choose **Delete** (the file goes to the Trash or Recycle Bin) or **Keep** (the file stays here and is restored on the other device).
- **Edited on both devices:** the newer edit keeps the name; the other is saved as a conflict copy on both devices.
- **Renames** are recognised and applied directly, with no re-download and no deletion question.
- File names Windows can't store (for example `notes: draft?.txt` made on a Mac) aren't synced, and are listed under **Needs attention**.
- Turn sync off with **Pause Sync** in the menu bar or tray, or in Settings.

### Browse and copy from the other computer

- Open the **Browse** tab: you'll see the other computer's **Home** folder and drives. Tick files or folders and click **Copy to Downloads**.
- It's read-only: the other computer can never change or delete anything on yours.
- Hidden and system folders (dot-files, `Library`, `AppData`) are never shown.
- Each computer chooses what it shares under **Settings → Browsing**: *Whole computer* (default), *Home folder*, *Sync folder* or *Nothing*.

### Transfers

- The **Activity** list shows progress. **Cancel** stops a transfer.
- **Retry** continues a failed or cancelled transfer from where it stopped, while the app is running.

### Background, login and notifications

- **Closing the window** keeps The Tunnel running in the menu bar (macOS) or system tray (Windows). Use the icon to open it, sync now, pause sync or quit.
- **Settings → General:**
  - *Open at login*
  - *"Send with The Tunnel" in the right-click menu*
  - *Notifications*

---

## Build from source

You need **Node.js 22 LTS** (20.19 or newer works), **Rust** (stable) and the platform's build tools. The steps below assume a fresh machine.

### Windows prerequisites

1. **Microsoft C++ Build Tools:** download *Build Tools for Visual Studio* from <https://visualstudio.microsoft.com/visual-cpp-build-tools/>. In the installer, tick **Desktop development with C++** and install.
2. **WebView2:** already included in Windows 11 and up-to-date Windows 10. If it's missing, install the *Evergreen Bootstrapper* from <https://developer.microsoft.com/microsoft-edge/webview2/>.
3. **Rust:** download and run `rustup-init.exe` from <https://rustup.rs> and accept the defaults (it uses the MSVC toolchain).
4. **Node.js:** install the **LTS** version from <https://nodejs.org>.
5. **Git:** <https://git-scm.com/download/win>.

Open a **new** PowerShell window afterwards so the new tools are on your `PATH`, and check:

```powershell
rustc --version
node --version
```

### macOS prerequisites

```sh
# Apple's command line tools (compiler, git)
xcode-select --install

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Node.js: install the LTS from https://nodejs.org, or with Homebrew:
brew install node@22
```

To build an Intel-compatible app on an Apple Silicon Mac (or the reverse), also run `rustup target add x86_64-apple-darwin aarch64-apple-darwin`.

### Get the code and run it

```sh
git clone https://github.com/<your-username>/<your-repo>.git
cd <your-repo>
npm install            # UI dependencies
npm run tauri dev      # run the app with live reload
```

The first run compiles all the Rust dependencies and takes a few minutes; later runs are quick.

Run the tests:

```sh
cargo test -p tunnel-core
```

### Build an installer

```sh
npm run tauri build
```

The results are in `target/release/bundle/`:

| System | Output |
| --- | --- |
| Windows | `nsis/The Tunnel_<version>_x64-setup.exe`, `msi/The Tunnel_<version>_x64_en-US.msi` |
| macOS | `macos/The Tunnel.app`, `dmg/The Tunnel_<version>_<arch>.dmg` |

> **macOS `.dmg` fails to build locally?** The DMG step uses AppleScript to lay out the Finder window, which your terminal may not be allowed to do. Build just the app with `npm run tauri build -- --bundles app`, or allow your terminal under **System Settings → Privacy & Security → Automation**.

<details>
<summary><b>Optional: build the Windows installer on a Mac</b></summary>

This is experimental, and only the `.exe` (NSIS) installer can be produced:

```sh
brew install llvm lld nsis
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
export PATH="$(brew --prefix llvm)/bin:$(brew --prefix lld)/bin:$PATH"
npx tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc
```

The installer is written to `target/x86_64-pc-windows-msvc/release/bundle/nsis/`.
</details>

---

## Publishing a release on GitHub

Two GitHub Actions workflows are included:

| Workflow | When it runs | What it does |
| --- | --- | --- |
| `.github/workflows/build.yml` | Every push and pull request | Runs the tests and builds the app on Windows and macOS. The installers are attached to the run under **Artifacts** (you need to be signed in to GitHub to download them). |
| `.github/workflows/release.yml` | When you push a tag starting with `v` | Builds every installer and attaches them to a **draft release**. |

To publish a release:

1. Make sure the version in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` is the one you want (for example `0.5.1`).
2. Tag that commit and push the tag:
   ```sh
   git tag v0.5.1
   git push origin v0.5.1
   ```
3. Wait for the **Release** workflow to finish (the **Actions** tab), about 15–20 minutes.
4. Open **Releases**, check the draft, and click **Publish release**. Anyone can now download the installers from the Releases page.

> If the workflow fails with a permissions error, open **Settings → Actions → General → Workflow permissions** in the repository and choose **Read and write permissions**.

---

## Troubleshooting

**The devices don't find each other**

1. Make sure both are on the **same** Wi-Fi or hotspot, and The Tunnel is open on both.
2. **Windows:** make sure you accepted the administrator prompt during installation. If not, reinstall, or allow The Tunnel when Windows Firewall asks, ticking **both Private and Public networks**.
3. **macOS:** check **System Settings → Privacy & Security → Local Network** and make sure The Tunnel is on.
4. Use **Connect by address** with the address shown at the bottom of the other device's **Settings**. If the other device is using a different port, type it as `address:port`, for example `172.20.10.3:47800`.
5. If even that fails, the network is blocking devices from talking to each other. Some phones and carriers isolate devices on a hotspot ("client isolation" or "AP isolation"). Try another phone's hotspot or a Wi-Fi router.

**macOS keeps asking for folder access after an update.** That's expected until the app is code-signed: macOS treats every new unsigned build as a new app.

**The Windows right-click entry is missing.** On Windows 11 it's under **Show more options**. Also check that *"Send with The Tunnel" in the right-click menu* is on in **Settings → General**.

**The macOS Quick Action is missing.** Open **System Settings → Keyboard → Keyboard Shortcuts → Services → Files and Folders** and make sure **Send with The Tunnel** is ticked. Also check that *"Send with The Tunnel" in the right-click menu* is on in The Tunnel's **Settings → General**, and that you opened the app installed in Applications at least once.

**A file isn't syncing.** Look under **Needs attention** on the **Sync** tab. Usually it's a file name Windows doesn't allow, or something that's a file on one device and a folder on the other.

**Where are the logs and settings?** See the data folders under [Uninstalling](#uninstalling).

---

## Uninstalling

**Windows:** **Settings → Apps → Installed apps → The Tunnel → Uninstall**. This also removes the firewall rule, the right-click entry and the login item.

**macOS:**
1. Quit The Tunnel from the menu bar icon.
2. Drag **The Tunnel** from Applications to the Trash.
3. Remove the login item and Quick Action:
   ```sh
   rm -f ~/Library/LaunchAgents/"The Tunnel.plist"
   rm -rf ~/Library/Services/"Send with The Tunnel.workflow"
   ```

**Settings, pairings and device identity** are kept in case you reinstall. Delete this folder to start completely fresh:

| System | Folder |
| --- | --- |
| Windows | `%APPDATA%\com.thetunnel.app` |
| macOS | `~/Library/Application Support/com.thetunnel.app` |

Your files, including the sync folder and received files, are never deleted by uninstalling.

---

## How it works

- **Discovery.** Each device announces itself on the local network with a small UDP broadcast (port 47801) and with mDNS/Bonjour (`_thetunnel._tcp`). If the network blocks both, you can connect by IP address. Announcements only provide an address to try; they are never trusted for identity.
- **Identity.** On first launch each device creates a private key and a self-signed certificate. The certificate's SHA-256 fingerprint is the device ID. Device and profile names are only labels.
- **Pairing.** The devices connect over mutually authenticated TLS 1.3 (port 47800). One commits to a random number, the other replies with its own, then the first reveals its number. Both derive the same 6-digit code from both certificate fingerprints and both random numbers. A device in the middle would produce different codes on each side, and the commitment stops it from guessing values until the codes match.
- **Authorization.** After pairing, a device only accepts transfers, browsing and sync requests from certificate fingerprints it has paired with.
- **Transfers.** Each file is streamed, checked against a SHA-256 hash on both ends, written to a temporary file, and only then moved into place. Partly received files are kept in a hidden `.tunnel-partial` folder so a retry can resume. Incoming paths are cleaned up (`..`, absolute paths, and names Windows can't store are rejected or rewritten), and nothing existing is ever overwritten.
- **Sync.**
  - Each device watches its sync folder and keeps an index: each file's content hash plus a *version vector*, which records how many times each device has changed it. Comparing vectors tells whether one copy is newer or both changed independently.
  - In each sync round a device fetches the other's index and pulls whatever is newer there; each device only ever changes its own folder.
  - Deletions are kept as tombstones and offered for review. A delete plus an add with the same contents is applied as a move.
  - Downloads go to `.thetunnel/tmp` inside the sync folder, resume after interruptions, and are only moved into place if the local file hasn't changed in the meantime.
- **Browsing.** Read-only. Hidden and system items and The Tunnel's own data are never listed or served, and symlinks aren't followed.

## Security and privacy

- **Local network only:**
  - Every incoming and outgoing connection is refused unless the address is private (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), link-local or loopback.
  - The interface loads nothing from the internet.
  - There are no accounts, analytics, telemetry or update checks.
- **Encrypted and authenticated:** all traffic uses TLS 1.3 with certificates pinned at pairing.
- **Only paired devices** can send you files, see your files or sync with you. Unpairing a device revokes this immediately.
- **You control sharing:** Settings → Browsing limits what paired devices can see; Settings → *Ask before receiving files* asks before anything arrives.

## Project layout

```text
crates/tunnel-core/     Rust engine: discovery, pairing, TLS, transfers, sync (shared with future mobile apps)
  src/sync/             Folder sync: index and version vectors, planning, rounds
  tests/                End-to-end tests (two engines talking over loopback)
src-tauri/              Desktop shell: commands, tray, login item, notifications, right-click menu
  windows/hooks.nsh     Windows installer hooks (firewall rule, clean uninstall)
src/                    React user interface
.github/workflows/      CI builds and release publishing
```

## Development notes

- **Run two copies on one machine** to test pairing and sync. Give each its own data folder and port:
  ```sh
  npx tauri build --debug --no-bundle
  TUNNEL_DATA_DIR=/tmp/tunnel-a TUNNEL_PORT=47820 target/debug/the-tunnel &
  TUNNEL_DATA_DIR=/tmp/tunnel-b TUNNEL_PORT=47821 target/debug/the-tunnel &
  ```
  On Windows PowerShell, set the variables with `$env:TUNNEL_DATA_DIR="C:\temp\tunnel-a"` first.
- **Headless discovery check:** `cargo run -p tunnel-core --example probe -- /tmp/probe` starts an engine and lists the devices it can see.
- **Network-dependent test** (real discovery and background sync): `cargo test -p tunnel-core --test sync -- --ignored`.
- **Release builds only:** the login item and right-click menu are only registered by release builds, so `npm run tauri dev` doesn't register itself.
- **Native look:** the UI follows each platform. macOS gets vibrancy, the system accent colour and Mac-style controls; Windows 11 gets Mica and Fluent-style controls. The styling lives at the end of `src/styles.css`.

## Status and roadmap

| Phase | Scope | Status |
| --- | --- | --- |
| 1 | Discovery, pairing, send and copy files and folders | ✅ Done |
| 2 | Automatic folder sync with deletion review, conflicts and rename detection | ✅ Done |
| 3 | Version history with restore | ⏸ Skipped for now |
| 4 | Menu bar / tray, open at login, notifications, right-click menu, cancel and resume, installers | ✅ Done |
| 5 | iPhone and Android apps | 🗓 Planned |
| — | Code signing (removes install warnings; needs paid Apple and Windows developer accounts) | 🗓 Planned |

Known limitations:

- Retry can't resume after the app restarts.
- On Windows 11 the right-click entry sits under *Show more options* until the app is signed.
