# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This crate is consumed as a git dependency pinned by tag, so a release here is
a tag its callers move onto deliberately. TokenGauge and TailGauge both pin it;
anything breaking is two repositories.

## [Unreleased]

### Fixed

- The README said TailGauge is GPL-3.0-or-later. Its `Cargo.toml` said so and
  its `LICENSE` file did not; the licence is permissive, and the paragraph
  explaining away a conflict that did not exist is gone.

### Changed

- Dual-licensed MIT OR WTFPL, matching TokenGauge. It was MIT; nothing is taken
  away.

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

[Unreleased]: https://github.com/Arzaroth/selvedge/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/Arzaroth/selvedge/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/Arzaroth/selvedge/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/Arzaroth/selvedge/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/Arzaroth/selvedge/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/Arzaroth/selvedge/compare/v0.1.1...v0.2.0
[0.1.0]: https://github.com/Arzaroth/selvedge/compare/v0.0.4...v0.1.0
[0.0.1]: https://github.com/Arzaroth/selvedge/releases/tag/v0.0.1
