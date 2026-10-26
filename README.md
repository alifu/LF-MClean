# LF-MClean

An interactive terminal cleaner for the caches of mobile developers on macOS: Xcode, Android, Gradle, Kotlin/Native,
CocoaPods and Swift Package Manager.

## Safety model

- Everything is moved to the **Trash** unless you pass `--permanent`. Simulators are the exception: `simctl`
  deletes them for good, and the review says so.
- Only a fixed list of cache folders inside your home folder is ever offered; every path is checked again
  just before it is removed.
- You review what is ticked before anything happens. Risky items (emulators, system images, archives)
  need you to type `delete`.
- The [keep list](#keeping-things) is checked last and wins over everything.

> **Status:** pre-1.0. The interactive cleaner, `purge`, the keep list and the removal log work.

## Usage

```bash
lfmclean                  # the menu: Clean, Packages, Purge, Keep list, History
lfmclean clean|packages|purge|keep|history   # open a screen directly
lfmclean clean --ids xcode.derived-data,gradle.caches --yes   # no screens, for scripts
lfmclean purge --paths ~/Code --json          # project build folders found, nothing removed
lfmclean --dry-run        # on any command: show what would be removed, remove nothing
lfmclean --permanent      # remove for good instead of moving to the Trash
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

## Keeping things

Whatever is on the keep list is never removed, whatever is ticked. The list lives in
`~/.config/lfmclean/keep.toml`; the screens edit it (`k` on a line), and it can be written by hand:

```toml
targets  = ["xcode.archives"]            # a whole cache, by id (ids from `lfmclean scan --json`)
packages = ["pods:Firebase", "spm:SnapKit@5.7.1"]   # every version, or one version
paths    = ["~/Library/Developer/Xcode/DerivedData/MyApp-abc", "/Volumes/Work/keep"]
```

- A missing file is an empty list. **A file that exists but cannot be read or understood blocks all
  removal** until it is fixed or deleted: running without it would remove the very things it protects.
  The same goes for an id that is not a cache lfmclean knows, including the `purge.*` ids (keep a
  project's folder with a path instead).
- A package keep (`pods:Name`, `spm:Name`) covers the copies in the package caches. It does **not**
  cover project checkouts, such as a `Pods/` folder inside a project; keep the project's folder (a
  path) for that.
- A kept symlink protects the link, not what it points to. Keep the target's path as well if both
  matter.

## The removal log

Every removal writes a line before it starts and another when it ends to
`~/Library/Logs/lfmclean/clean.log`; `lfmclean history` (or the History screen) shows them. A removal
that started but never ended, such as one cut short by a crash, is shown as **interrupted**.

A `--dry-run` removes nothing and so writes nothing to the log.

If background work (a scan, a removal) stops because of a bug, the screens keep running and say so;
the details are written to `~/Library/Logs/lfmclean/crash.log`. Please include it in a bug report.

## Notes

- Running Xcode, Android Studio, an emulator or a Gradle daemon is checked with `ps`, and a warning
  names the caches each one uses. If `ps` cannot be run, the review says the check failed instead of
  staying quiet.
- `purge` looks for projects in `~/Documents`, `~/Developer` and `~/Projects` (or `--paths`). It does
  not enter other volumes, hidden folders, or the folder `GRADLE_USER_HOME` names, and gives up on a
  folder after a minute. A `build` folder with files that git tracks, or with content Gradle does not
  generate, is offered as **caution**, not as safe.
- The terminal must be at least 60 columns by 12 rows; a smaller window shows a request to enlarge it.

## Install

```bash
brew install alifu/tap/lfmclean
brew upgrade lfmclean
brew uninstall lfmclean
```

Or download `lfmclean-<version>-macos-universal.tar.gz` from the
[releases](https://github.com/alifu/LF-MClean/releases) page (check it against the `.sha256` file) and put
`lfmclean` on your `PATH`. The binary is ad-hoc signed, not notarized; Homebrew's download is not
quarantined, so it runs at once. If you download it in a browser, run
`xattr -d com.apple.quarantine lfmclean` once.

## Build

```bash
cargo build
cargo test
cargo run
```

## License

MIT, see [LICENSE](LICENSE).
