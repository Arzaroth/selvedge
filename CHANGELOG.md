# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This crate is consumed as a git dependency pinned by tag, so a release here is
a tag its callers move onto deliberately. TokenGauge and TailGauge both pin it;
anything breaking is two repositories.

## [Unreleased]

### Added

- **`plans::team_seat`: which seat a Claude Team login holds.** A rate-limit
  tier naming an `Nx` makes it premium whatever the seat tier is called - a
  premium seat reports `team_tier_1` beside `default_claude_max_5x` - and
  `team_standard` / `team_premium` decide otherwise. `Seat::weight`,
  `TEAM_STANDARD_WEIGHT`, `TEAM_PREMIUM_WEIGHT` and `tier_multiplier` come
  with it, so TokenGauge and remuda stop each carrying the rule.

## [0.7.1] - 2026-10-11

### Fixed

- **A zip that lists its directories unpacks.** self_update's extractor
  created a file for every entry, so a release zip built by 7-Zip failed on
  its first directory and no Windows update could succeed. Zips are unpacked
  here now, and an entry whose name leads outside the staging directory is
  refused rather than written there.
- **A staged binary must be a regular file.** A symlink in the archive was
  installed as a symlink, and its target made executable.

### Added

- **An MSI owns only the copy in its directory.** Beside the ProductCode, the
  marker key may hold `InstallDir`; a copy running from anywhere else is not
  the MSI's. A marker without it covers every copy, as before.

## [0.7.0] - 2026-10-11

### Added

- **Who owns the binary, before anything is fetched.** `update::owner`
  returns the `Owner` an update must not go around, now public, and
  `Owner::advice` says how to upgrade instead. Besides Homebrew and an app
  bundle it knows a distribution's package (under `/usr` outside
  `/usr/local`, named by the dpkg, rpm or pacman database it finds), the Nix
  store, a snap, and the project's MSI. `apply` refuses the new ones too:
  replacing a file a package database lists leaves it describing a version
  that is gone.
- **Archives that wrap their files in a directory.** An archive holding one
  directory named after the asset, `<name>-v1.2.3-<target>/`, is read from
  inside it.
- **Windows on arm64**: `windows-aarch64` in `SHORT_TARGETS`.
- **`updater`: the updater without a TLS provider.** `self-update` links
  aws-lc through self_update's rustls feature; `updater` leaves the provider
  to the caller, who installs one as the process default, so a binary built on
  ring links one crypto library rather than two.

### Changed

- **A project names its own platforms.** `Project` gains `targets`, how each
  platform is spelled in the release's asset names, so a project releasing
  under Rust target triples can update too. Both callers set
  `targets: selvedge::SHORT_TARGETS` to keep `<os>-<arch>`.
  `update::arch_target` takes the project.

## [0.6.0] - 2026-10-10

### Added

- **`plans`: several subscriptions added up into one meter.** `pool` counts
  each plan in units of the largest ("105% of 105%" for a spent Max 20x and
  Pro), gives the pooled fraction a bar fills to, and each plan's split-bar
  segment, a tenth of the bar at least. `whole_percent` rounds a half to even,
  as `{:.0}` does. TokenGauge and remuda each had a copy, one of them in
  TypeScript. Weights may be fractional: a Claude Team seat is 1.25 Pros.

## [0.5.0] - 2026-09-29

### Added

- **An app bundle is replaced whole.** `apply` refuses when the running binary
  sits under a `.app/Contents/` and points at the latest release to download
  instead, whether it was launched from the bundle or through a symlink inside
  it. A binary swapped inside a signed bundle breaks its seal, and macOS then
  reports the whole app as damaged.

## [0.4.0] - 2026-09-25

### Added

- **macOS.** `arch_target` answers `macos-x86_64` and `macos-aarch64`. It used
  to answer `linux-*` for anything that was not Windows, so a Mac running an
  update would have downloaded the Linux tarball and installed ELF binaries
  over itself. Any other platform is now refused rather than handed Linux.
- **A Homebrew install is left to Homebrew.** `apply` refuses when the running
  binary resolves into a `Cellar`, and says to run `brew upgrade` instead.
  Replacing the keg's files in place leaves brew describing a version that is
  no longer on disk. A binary whose own path cannot be resolved is refused too,
  since that unresolved path is the brew symlink.

### Fixed

- The README described the callers' licence terms. It said TailGauge is
  GPL-3.0-or-later, which its `Cargo.toml` said and its `LICENSE` file did not.
  What another repository is licensed under is that repository's to state, so
  the claim is gone rather than corrected.

### Changed

- Dual-licensed MIT OR WTFPL, matching TokenGauge. It was MIT; nothing is taken
  away.

## [0.3.2] - 2026-09-16

### Added

- `update::silence_progress()` stops downloads drawing a progress bar. The bar
  goes to stdout, which is right for a command line and wrong for a terminal UI
  that owns the screen: it was written across the last frame the UI painted.

## [0.3.1] - 2026-09-16

### Fixed

- **An update could install binaries for another platform.** `asset_for` looks
  for the running target and then falls back to any asset whose name merely
  carries the archive suffix, so a release whose aarch64 build failed handed an
  aarch64 machine the x86_64 tarball and the update succeeded. A release that
  does not carry this platform is now refused as one.

### Changed

- `Applied` derives `Debug`.

## [0.3.0] - 2026-09-16

### Added

- `state::announce_if_new`, which fires a one-shot announcement for a version
  not yet announced. The guard, the claim and the write-back happen under the
  lock a check and an update already take; the caller supplies only the
  wording. A caller doing this for itself was the one writer of that file not
  participating in the lock, and an update finishing mid-announcement was
  undone on paper.
- `proc`: `which`, `has`, `run`, `run_quiet` and `output`. `which` answers only
  for a file that is executable, which the copies in both callers did not.

## [0.2.2] - 2026-09-16

### Fixed

- A check read the cache, spent seconds asking GitHub, then wrote what it had
  decided before the call, over a file `apply` writes under a lock the check
  never took. An update finishing inside that window was undone, and the cache
  went on offering an update to the version already installed.

## [0.2.1] - 2026-09-16

### Fixed

- The release asset and the extractor follow the platform. `arch_target`
  matched on the architecture alone, so an x86_64 Windows build asked for the
  `linux-x86_64` asset, and `ARCHIVE_SUFFIX` and the extractor were hardcoded
  to gzipped tar beside it.

## [0.2.0] - 2026-09-16

### Added

- A `self-update` feature, on by default. Without it the payload installer and
  the cached state arrive with no network stack behind them, which is what a
  GUI reading a cached check needs.

## [0.1.0] - 2026-09-16

### Changed

- Generalised to fit a second caller: two binaries where there was one, aliases
  an update has to keep pointing at the new copy, and an MSI-owned install on
  Windows.

## [0.0.1] - 2026-09-16

### Added

- The updater and the desktop payload installer, extracted from TailGauge.

[Unreleased]: https://github.com/Arzaroth/selvedge/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/Arzaroth/selvedge/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Arzaroth/selvedge/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/Arzaroth/selvedge/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/Arzaroth/selvedge/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/Arzaroth/selvedge/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/Arzaroth/selvedge/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/Arzaroth/selvedge/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/Arzaroth/selvedge/compare/v0.1.1...v0.2.0
[0.1.0]: https://github.com/Arzaroth/selvedge/compare/v0.0.4...v0.1.0
[0.0.1]: https://github.com/Arzaroth/selvedge/releases/tag/v0.0.1
