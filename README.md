# The Tunnel

Move files between your Mac and Windows PC over the same Wi-Fi or phone hotspot.
Files go directly from one device to the other, encrypted, and never touch the internet.

## Status

**Version 0.2:** phases 1 and 2 are done: discovery, pairing, send and pull, and automatic folder sync.

| Phase | Scope |
| --- | --- |
| 1 | Find devices, pair with a 6-digit code, send and pull files and folders ✅ |
| 2 | Auto-sync of The Tunnel folder: change detection, rename handling, delete review, conflict copies, resumable downloads ✅ |
| 3 | Version history with restore |
| 4 | "Send to my Mac/PC" in the right-click menu, start at login, signed installers |
| 5 | iOS and Android |

## How it works

- **Discovery.** Each device announces itself with a UDP broadcast beacon (port 47801) and with mDNS/Bonjour (`_thetunnel._tcp`). If a hotspot blocks both, you can connect by IP address.
- **Identity.** On first launch each device creates a private key and a self-signed certificate. The certificate's SHA-256 fingerprint is the device ID. Device and profile names are only labels.
- **Pairing.** The devices connect over mutually authenticated TLS 1.3. One device commits to a random nonce, the other replies with its own nonce, then the first reveals its nonce. Both derive a 6-digit code from the two fingerprints and both nonces, and the users check that the codes match. A machine in the middle would produce different codes, and the commitment stops it from guessing values until the codes happen to match.
- **Authorization.** After pairing, a device accepts transfers, folder listings and pulls only from a certificate fingerprint it has paired with.
- **Local only.** Inbound and outbound connections are refused unless the address is private (10/8, 172.16/12, 192.168/16), link-local or loopback. The UI loads no remote resources, and the app has no telemetry or update checks.
- **Safe writes.** Incoming paths are sanitized, so `..`, absolute paths and Windows-reserved names are rejected or rewritten. Each file is written to a `.tunnelpart` file, checked against the sender's SHA-256 hash, and only then renamed into place. Existing files are never overwritten: a second `Photos` arrives as `Photos (1)`.
- **Sync.** Each device watches The Tunnel folder and keeps an index of it. Each file in the index has a content hash and a version vector, which records how many times each device has changed it. In each round a device fetches the other device's index and pulls whatever is newer there. Deletions are never applied automatically: the other device asks you to *Delete* (the file goes to the Trash or Recycle Bin) or *Keep* (the file is restored on the deleting device). A delete plus an add with the same contents is applied as a rename. If both devices edit the same file, the later edit keeps the name and the other is saved as `name (conflict from <device> <time>).ext`. Downloads go to `.thetunnel/tmp` inside the folder, resume after an interruption, and are moved into place only after their hash checks out and the local file hasn't changed in the meantime. Names Windows can't store are skipped and reported.
- **Pulling.** A paired device can browse and copy files only from your **shared folder** (default `~/The Tunnel`). Symlinks are not followed.

## Project layout

```
crates/tunnel-core/   Rust engine (discovery, pairing, TLS, transfers); shared with future mobile apps
src-tauri/            Desktop shell: exposes the engine to the UI as Tauri commands
src/                  React UI
```

## Development

Requirements: Node 20+, Rust (via rustup). On Windows you also need the Visual Studio C++ Build Tools and WebView2, which Windows 11 already includes.

```sh
npm install
npm run tauri dev               # run the app with hot reload
cargo test -p tunnel-core       # engine tests (pairing, transfers, path safety)
npm run tauri build             # build an installer for the current OS
```

To run two copies on one machine (useful for testing pairing), give each copy its own data folder and port:

```sh
npx tauri build --debug --no-bundle
TUNNEL_DATA_DIR=/tmp/tunnel-a TUNNEL_PORT=47820 target/debug/the-tunnel &
TUNNEL_DATA_DIR=/tmp/tunnel-b TUNNEL_PORT=47821 target/debug/the-tunnel &
```

`cargo run -p tunnel-core --example probe -- /tmp/probe` starts a headless engine and lists the devices it can see.

The GitHub Actions workflow in `.github/workflows/build.yml` runs the engine tests and builds the Windows (`.msi`, `.exe`) and macOS (`.dmg`) installers on every push.

## Platform notes

- **Windows firewall.** The first time The Tunnel runs, Windows asks whether to allow it on the network. Allow it. A newly joined hotspot is often marked "Public", so either tick *Public networks* or switch the network to *Private*.
- **macOS Local Network.** macOS asks for permission to find devices on the local network. Allow it.
- **Phone hotspots.** Some phones and carriers isolate devices on the hotspot from each other. If neither discovery nor *Connect by address* works, the hotspot is blocking device-to-device traffic. Try another phone, or a Wi-Fi router.
