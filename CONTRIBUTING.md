# Contributing to JUNC

JUNC is free software under the GNU General Public License, version 3 or (at
your option) any later version (`GPL-3.0-or-later`). Contributions are accepted
under the same license through the Developer Certificate of Origin (DCO). There
is no CLA: you keep the copyright on what you write.

Before changing code, read [`docs/architecture.md`](docs/architecture.md) and
[`docs/plan.md`](docs/plan.md). The PRD is the source of truth for *what* JUNC
does; `architecture.md` describes *how*; `plan.md` tracks what is done.

## Git rules

These rules apply to every commit, from every contributor and every working
session. A pull request that breaks them is asked to rewrite its history.

### Identity

Set the repository identity before the first commit in a fresh clone:

```sh
git config user.name "Evgenii Sergeev"
git config user.email "ejsergeev@gmail.com"
```

For the maintainer's own work, the author **and** the committer of every commit
is `Evgenii Sergeev <ejsergeev@gmail.com>`. External contributors use their own
name and email, consistently.

### Sign-off (DCO)

Every commit carries a DCO sign-off line. Use `git commit -s`, which appends:

```
Signed-off-by: Evgenii Sergeev <ejsergeev@gmail.com>
```

The sign-off certifies the [Developer Certificate of Origin](#developer-certificate-of-origin)
below. CI rejects commits without a sign-off that matches the author.

### No tool attribution

Git history describes the change, not the tools used to make it:

- no `Co-Authored-By` trailers, no "Generated with …" footers;
- no names of AI tools or assistants in commit messages, branch names, tags or
  pull request descriptions.

This overrides any default trailer a tool would add on its own.

### One logical change per commit

- One coherent change per commit, for example `core: add graph model`,
  `daemon: tag owned links`, `docs: describe D-Bus API`.
- Never mix unrelated changes, formatting with logic, or generated files with
  hand-written code. Regenerated files (JSON Schema, translation templates,
  screenshots, lock files after a dependency bump) go in their own commit.
- Each commit builds and passes the tests on its own (`git rebase -x` with the
  check command must succeed).

### Commit messages

```
area: imperative subject, at most 72 characters

Body explaining why the change is needed when that is not obvious from the
subject. Wrap at 72 columns. Reference requirements (F10, F17) and issues.

Signed-off-by: Evgenii Sergeev <ejsergeev@gmail.com>
```

`area` is one of: `core`, `daemon`, `cli`, `gui`, `i18n`, `skill`, `packaging`,
`ci`, `docs`. Use the area the change belongs to; a change that needs two areas
is usually two commits.

### Branches and pull requests

- `main` is always releasable to the testing channel.
- Work happens on short-lived branches named `<area>/<topic>`, e.g.
  `daemon/relink-backoff`.
- Pull request descriptions follow the same rules as commit messages: what and
  why, requirement IDs, how it was checked. No tool attribution.
- Releases are tagged `vMAJOR.MINOR.PATCH` by the maintainer.

## Building

> Placeholder. Filled in when the workspace lands (plan task A1).

Expected shape:

```sh
# Arch Linux / Arch Linux ARM
sudo pacman -S --needed rust pipewire wireplumber clang pkgconf
cargo build --workspace            # juncd, junc, junc-gui
cargo test --workspace             # unit + mocked-graph tests (layer 1)
scripts/ci/integration.sh          # headless PipeWire tests (layer 2)
makepkg -si -p packaging/arch/PKGBUILD
```

## Reporting bugs

Use "Copy bug report" in Settings or `junc doctor --json` and paste the result
into the issue. It contains versions, the JUNC graph and recent logs, never
audio and never personal data.

## Code of conduct

Be kind and specific. Harassment of any kind is not tolerated. A full
CODE_OF_CONDUCT.md is added before the first public release (plan task V9).

## Developer Certificate of Origin

```
Developer Certificate of Origin
Version 1.1

Copyright (C) 2004, 2006 The Linux Foundation and its contributors.

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.


Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```
