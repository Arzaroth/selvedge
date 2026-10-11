# selvedge

The self-finished edge of a piece of cloth is the bit that stops it
unravelling. This is the same idea for a desktop widget that ships as a binary
plus the QML and JavaScript that binary feeds: it keeps them from coming apart.

Two projects, [TailGauge](https://github.com/Arzaroth/TailGauge) and
[TokenGauge](https://github.com/Arzaroth/TokenGauge), draw a panel on Plasma,
GNOME and Omarchy by shelling out to a binary. TokenGauge ships a few more of
them - a TUI, a tray, a waybar module - which is why a project here names its
binaries rather than its binary. Neither panel has anything to do with the
other. What they share is everything around the panel: fetching a release from
GitHub, replacing the running binary, and reinstalling the desktop payloads
from the same archive so the binary and its frontends can never be a release
apart.

That was 1,400 lines in each of them.

## What a caller provides

Everything the machinery cannot know, in one value:

```rust
const TAILGAUGE: selvedge::Project = selvedge::Project {
    // Primary first: it names the release assets and every alias points at it.
    binaries: &["tailgauge"],
    repo: "Arzaroth/TailGauge",
    repo_env: "TAILGAUGE_REPO",
    version: env!("CARGO_PKG_VERSION"),
    frontends: FRONTENDS,
    // Old names the binary still answers to, kept as symlinks.
    aliases: ALIASES,
    // Old names it does not, removed rather than left to answer.
    legacy: &["tailgauge-update"],
    // How each platform is spelled in the asset names: `linux-x86_64` here,
    // or a table of Rust target triples for a project releasing under those.
    targets: selvedge::SHORT_TARGETS,
    // Windows only: where an MSI records its ProductCode. None means every
    // update replaces in place.
    msi_marker_key: None,
};
```

`version` has to come from the caller: `env!("CARGO_PKG_VERSION")` inside this
crate would report this crate's version, which is never what anyone wants.

Then:

```rust
let status = selvedge::update::check_cached(&TAILGAUGE, &cache, force)?;
let applied = selvedge::update::apply(&TAILGAUGE, &cache)?;
```

A release archive may hold its files at the top or inside one directory named
after the asset (`tailgauge-v1.2.3-linux-x86_64/`); either is read.

## Who owns the copy on disk

An update replaces the files beside the running binary, which is only right
when nobody else keeps a record of them. `update::owner` says who does,
without fetching anything: Homebrew, a macOS app bundle, a distribution's
package manager (anything under `/usr` outside `/usr/local`, named after the
package database it finds), the Nix store, a snap, or the project's MSI.
`apply` refuses all of them with `Owner::advice`, which says how to upgrade
instead, except the MSI, which it hands the upgrade to. A caller that wants to
refuse the MSI too, or to refuse before reaching GitHub, asks `owner` first.

## Several plans in one meter

`selvedge::plans` is the other thing the projects share: adding several
subscriptions at one vendor up into one meter. TokenGauge draws it in its
panels and remuda on its page, and each used to carry its own copy of the
arithmetic, in Rust and in TypeScript.

```rust
use selvedge::plans::{pool, Share};

let p = pool(&[
    Share { used: 100.0, weight: 20.0 }, // a Max 20x, spent
    Share { used: 100.0, weight: 1.0 },  // a Pro, spent
]).unwrap();
assert_eq!(p.label(), "105% of 105%");
```

Plans count in units of the largest one, a half rounds to even, a pooled bar
fills to `Σ used × weight ÷ Σ weight`, and a split bar gives each plan the
stretch its weight buys, a tenth of the bar at least. Only the numbers are
here: the label, the tone and any "estimate" badge are the caller's. It
needs no feature and links nothing.

## The lock

An update takes an exclusive lock on the install directory so one fired from
the panel and one fired from a terminal cannot race on the same staging
directory. The lock is the kernel's, held on an open descriptor, rather than
the existence of a file: a process that is killed closes its descriptors, and
the next update proceeds. A lock file that means "locked" while it merely
exists survives the crash and blocks every later update until somebody deletes
it by hand.

## Features

`self-update` is on by default and is the half that links a network stack:
fetching a release, replacing the binary. A caller that only installs payloads
and reads a cached check - a GUI that shells out to whoever owns the network -
takes the crate with `default-features = false` and still gets `frontend` and
`state`.

`self-update` brings its own TLS provider, aws-lc. A caller already built on
another one - ring, say, through iroh or quinn - takes `updater` instead, which
is the same updater with no provider of its own, and installs its provider as
the process default before calling it.

## Windows

A project installed by an MSI is upgraded by the MSI, because replacing the
files underneath it would leave Windows describing a version that is no longer
installed. The installer records its ProductCode under `msi_marker_key`; if it
also records `InstallDir` there, only a copy running from that directory
counts as the MSI's. `apply` returns with `installer_launched` set and nothing replaced,
and the caller must exit promptly: one of the files `msiexec` is about to
replace is the executable running the code.

That path compiles nowhere but Windows, so CI builds and tests there too. The
registry parsing it depends on is deliberately not behind `cfg`, because that is
where the mistakes are and a test that only runs on a release runner is a test
nobody sees fail.

## Tests

The suite drives the whole thing against `SAMPLE`, a project that does not
exist, which is what keeps this crate honest about being machinery rather than
one of its callers wearing a different name.

`tests/callers.rs` is the other half: both real projects, declared the way they
declare themselves, and the call sites they use today. The crate came out of
one of them and fits that one by construction. TokenGauge ships two binaries
where TailGauge ships one, keeps an old name working where TailGauge takes one
away, and is MSI-installed on Windows where TailGauge has no Windows at all - so
if the surface moves under either of them, that file stops compiling.

## Provenance and licence

selvedge is dual-licensed: use it under **either** the
[MIT License](LICENSE-MIT) **or** the [WTFPL](LICENSE-WTFPL), whichever you
prefer (`SPDX-License-Identifier: MIT OR WTFPL`), matching both callers.

The code was extracted from [TailGauge](https://github.com/Arzaroth/TailGauge)
and merged with the equivalent half of
[TokenGauge](https://github.com/Arzaroth/TokenGauge). Every line that came here
was written by one person, who holds the copyright in it and licenses it here
under these terms. Nothing was taken from a contributor whose terms this would
have to respect, and what either caller is licensed under is a question for
that repository rather than a claim worth mirroring here.

No third-party code is vendored. Every dependency is permissive - MIT,
Apache-2.0 or dual - and none is copyleft.
