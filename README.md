# selvedge

The self-finished edge of a piece of cloth is the bit that stops it
unravelling. This is the same idea for a desktop widget that ships as a binary
plus the QML and JavaScript that binary feeds: it keeps them from coming apart.

Two projects, [TailGauge](https://github.com/Arzaroth/TailGauge) and
TokenGauge, draw a panel on Plasma, GNOME and Omarchy by shelling out to a
binary. Neither panel has anything to do with the other. What they share is
everything around the panel: fetching a release from GitHub, replacing the
running binary, and reinstalling the desktop payloads from the same archive so
the binary and its frontends can never be a release apart.

That was 1,400 lines in each of them.

## What a caller provides

Everything the machinery cannot know, in one value:

```rust
const TAILGAUGE: selvedge::Project = selvedge::Project {
    binary: "tailgauge",
    repo: "Arzaroth/TailGauge",
    repo_env: "TAILGAUGE_REPO",
    version: env!("CARGO_PKG_VERSION"),
    frontends: FRONTENDS,
    aliases: ALIASES,
    legacy: &[],
};
```

`version` has to come from the caller: `env!("CARGO_PKG_VERSION")` inside this
crate would report this crate's version, which is never what anyone wants.

Then:

```rust
let status = selvedge::update::check_cached(&TAILGAUGE, &cache, force)?;
let applied = selvedge::update::apply(&TAILGAUGE, &cache)?;
```

## The lock

An update takes an exclusive lock on the install directory so one fired from
the panel and one fired from a terminal cannot race on the same staging
directory. The lock is the kernel's, held on an open descriptor, rather than
the existence of a file: a process that is killed closes its descriptors, and
the next update proceeds. A lock file that means "locked" while it merely
exists survives the crash and blocks every later update until somebody deletes
it by hand.

## Tests

The suite drives the whole thing against `SAMPLE`, a project that does not
exist, which is what keeps this crate honest about being machinery rather than
one of its callers wearing a different name.
