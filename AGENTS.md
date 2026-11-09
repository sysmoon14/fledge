# Repository Guidelines

## Project Structure & Module Organization

This Rust 2021 workspace builds USB recovery tools for Nest thermostats:

- `crates/fledge-core`: firmware parsing, patching, and recovery logic.
- `crates/fledge-host`: USB workflows, embedded assets, and host orchestration.
- Device-specific code sits in one module per device (`diamond3`, `j49`) in core and host, plus host's detection-only `onyx` (the Thermostat E); `fledge-host::device` identifies the unit on USB by its identity in any mode and the run dispatches to its device's flow; the gen 1 and gen 2 share their boot ROM's identity, and in that mode the OMAP flow reads the generation from the NAND's U-Boot environment (`nlmodel`) once the flasher runs.
- `crates/fledge-gui` and `crates/fledge-cli`: the `fledge` executable (egui window) and the terminal interface, a library it runs when given arguments on Linux and macOS; Windows has the window only.
- `crates/nestlocal`: device service implementing MQTT, Matter, and Zigbee; startup scripts live in `rootfs/`.
- `platform/`: what is built for and installed on the thermostat, by device:
  - `common/`: both generations. `nlclient/` (native settings screen, assets, binary discovery references, C tests), `radio/` (radio firmware, each radio's board, linker and OpenThread files in `em3581/` and `em357/`; OpenThread integration in `thread/`), `doom/` (doomgeneric submodule plus its platform layer).
  - `diamond3/`: the gen 3. `bootstrap/` (ROM stage and boot wrapper), `uboot/` (recovery U-Boot submodule, patches and build scripts), `initramfs-init`, `zbwake/` (kernel wake module for each gen 3 release).
  - `j49/`: the gen 2. x-loader build and bare-metal NAND flasher, loaded through x-loader's USB loader, its USB-serial kernel, `zbwake/` module, and dropbear for `--ssh`.
- `utils/`: `build-firmware.sh` builds all of `platform/` into `build/`; `make_macos_app.sh` and `install_linux.sh` package the app. `ci/` holds the build image's Dockerfile.
- `assets/`: app icons, the Linux `.desktop` entry, and the README's app screenshot and overview diagram (light and dark).
- `integrations/`: files users install on their side: Home Assistant blueprints (`home-assistant/`) and the Zigbee2MQTT converter (`zigbee2mqtt/`).
- `.github/workflows/`: `checks.yml` runs nestlocal's tests and lints on every push and pull request; `apps.yml` builds the firmware and the app for every platform into each published release; `image.yml` builds the image both run in and pushes it to this repository's registry (`ghcr.io/sysmoon14/fledge-build`), run by hand when `utils/ci/Dockerfile` changes.

Generated outputs belong in ignored `build/`, `target/`, `dist/`, or `.work/` directories.

User documentation for the app, its options and integrations is README.md; update it when an option, its app label, or integration behavior changes. nestlocal's MQTT topics are documented in `crates/nestlocal/src/bridge.rs`; the home screen cards' protocol is `crates/nestlocal/src/ui/` with the Fledge integration (sysmoon14/fledge-homeassistant).

## Build, Test, and Development Commands

Run commands from the repository root:

```sh
git submodule update --init
utils/build-firmware.sh
cargo build --release -p fledge-gui
cargo run -p fledge-gui -- --help
cargo test --workspace --locked
sh utils/make_macos_app.sh
```

The firmware build supplies assets embedded by the host crates; cargo runs it when `build/` lacks them, except on Windows and in CI. Run it yourself after changing firmware sources. macOS uses Docker for firmware compilation. See `BUILDING.md` for platform dependencies. The packaging command produces `dist/Fledge.app`.

## Coding Style & Naming Conventions

Match surrounding style and use four-space indentation in Rust. Use `snake_case` for functions/modules, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Keep changes focused; avoid unrelated formatting or refactoring. Comments explain current behavior and lasting constraints, not debugging history.

## Testing Guidelines

Rust tests use `#[test]` and integration tests under `crates/fledge-core/tests/`. Name tests after observable behavior. CI also runs:

```sh
clang -std=c11 -Wall -Wextra -Werror platform/common/nlclient/tests/custom_flow_test.c -o /tmp/custom-flow-test
/tmp/custom-flow-test
python3 -m pytest platform/common/radio/tests
cargo clippy --workspace --locked --all-targets -- -D warnings
```

For binary patch changes, follow `platform/common/nlclient/tests/README.md`; strict golden tests require private firmware samples. Report hardware verification separately from host tests. No numeric coverage threshold is specified.

## Commits and PR titles

    subsys: lowercase imperative summary

Everything after the colon is **lowercase**. The subject fits in 72 columns, no trailing period. The subsystem prefix matches the area the change touches — check `git log --format=%s -20 -- <path>` for precedent. Common prefixes:

| Prefix | Area |
|-|-|
| `app:` | the Fledge application (GUI, CLI, self-update, host library) |
| `nestlocal:` | the on-device service; nest deeper with `nestlocal: thread:` |
| `client-ng:` | the thermostat UI submodule |
| `ci:` | GitHub Actions workflows and build infrastructure |
| `radio:` | radio firmware and Thread/Zigbee protocol |
| `j49:` | gen 2 device-specific code |
| `platform:` | build scripts, bootstrap, x-loader, kernel modules |
| `docs:` | documentation only |

Never `WIP`, `fix`, or bare verbs without a subsystem prefix.

Body is optional — omit it for self-explanatory changes. When present, wrap at 72 columns, at most three lines: why the change is needed and any non-obvious choice.

- One logical change per commit, and every commit builds and passes its tests on its own. Fold review fixes into the commit that owns the lines instead of adding fixup commits; without an interactive terminal: `git commit --fixup=<sha>`, then `GIT_SEQUENCE_EDITOR=true GIT_EDITOR=true git rebase -i --autosquash origin/main`.
- Update a branch with `git rebase origin/main`, never by merging `main` into it.
- Stage explicit paths (`git add <file>`) rather than `git add -A` or `commit -a`, which pick up stray files. Before pushing, review `git show --stat origin/main..HEAD`.
- Branches: `feature/<short-name>` for features, `bugfix/<short-name>` for fixes.

## Pull requests

- A PR is one self-contained change. PRs are squash-merged: the PR title becomes the commit subject on `main` (with ` (#N)` appended), so it must follow the `subsys: lowercase summary` format above.
- PR body is short — one or two sentences describing the change. No padding.
- CI (`.github/workflows/`) runs nestlocal's tests and lints on every push and pull request; a PR merges only when it is green. Publishing a release (`utils/release-nestlocal.sh`) builds the firmware and every app target into it. The workflows pin the build image by digest: after changing `utils/ci/Dockerfile`, run the Build image workflow and pin the digest it reports in `apps.yml` and `checks.yml`.
- Never describe an undisclosed vulnerability in a public issue, PR or commit.

## Security & Device Configuration

Do not commit firmware dumps, credentials, pairing keys, or private device logs. Preserve saved network credentials during device testing. Thread and Zigbee require exclusive ownership of the onboard radio; document the final device configuration after hardware tests.
