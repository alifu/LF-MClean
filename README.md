# LF-MClean

An interactive terminal cleaner for the caches of mobile developers on macOS: Xcode, Android, Gradle, Kotlin/Native,
CocoaPods and Swift Package Manager.

> **Status:** early development. `lfmclean scan` lists the caches it knows about and their sizes.
> Nothing is removed yet: the interactive cleaner comes next.

## Usage

```bash
lfmclean scan             # caches found and their sizes, grouped like Xcode / Android / Gradle & Kotlin
lfmclean scan --items     # the same, with every item listed under its cache
lfmclean scan --json      # everything as JSON, with the paths of each item
```

`scan` only reads. A size ending in `+` is a lower bound: some folders could not be read. Sizes are
the disk space taken (like Finder), in powers of 1000.

What it looks at (all inside your home folder, nothing else is ever offered):

| Group | Caches |
|---|---|
| Xcode | DerivedData, device support files, SwiftUI preview data, simulator caches, unavailable simulators, archives, Swift package clones and manifests, CocoaPods cache and spec repos |
| Android | Android Studio and JetBrains caches and logs, emulators, emulator system images |
| Gradle & Kotlin | Gradle caches, daemon logs and distributions, Kotlin/Native toolchains and dependencies |

Archives, emulators and system images are marked **risky**: they hold something that cannot be made again.

## Install

Planned (after v1.0.0):

```bash
brew install alifu/tap/lfmclean
brew upgrade lfmclean
brew uninstall lfmclean
```

## Build

```bash
cargo build
cargo test
cargo run
```

## License

MIT, see [LICENSE](LICENSE).
