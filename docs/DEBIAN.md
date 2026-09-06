# Debian packages

## Availability

Debian packaging is implemented in the current source tree. Published Omnivox
1.7.1 predates this work and has no `.deb` asset. Use a development package
until a subsequent release includes the package and its checksum. Remote
workstation speech remains preview, including when installed from a package.

## Release packages

The release workflow builds `omnivox_VERSION-1_amd64.deb` on Ubuntu 24.04 and
checks installation, speech synthesis, upgrades, and removal in clean Ubuntu
24.04 and 26.04 containers. The package is included in `sha256sums.txt` and
uploaded to the draft GitHub release alongside the portable archives. Both
Ubuntu checks repeat against the downloaded draft package before publication.
They verify the binary version and recorded source commit against the release
tag, as well as the package checksum and installed files.

Download the package and `sha256sums.txt` from the same release. Verify it before
installing (replace `VERSION` with the release version):

```sh
sha256sum --ignore-missing --check sha256sums.txt
sudo apt install ./omnivox_VERSION-1_amd64.deb
```

The release also includes `omnivox-VERSION-piper-source.tar.gz`. Despite its
historical name, this archive contains the complete tagged Omnivox source tree,
all locked Cargo dependency sources, and the matching eSpeak NG and Sonic
sources used by the core server, as well as Piper's inputs. The package's
`BUILD-INFO.json` identifies this corresponding-source archive. Its existing
source verification gate remains required before publishing the release.
The [licensing policy](LICENSING.md) describes the component boundaries.

`make package-deb-release` (or `python3 tools/package_deb.py --release`) builds
the release candidate locally. It requires a clean source tree at the exact
`vVERSION` tag, a stable workspace version, and Debian revision `1`. In CI,
`GITHUB_REF` must also name that tag. It never creates a tag or uploads anything.
Normal branch and pull-request CI builds exercise development packaging and
both Ubuntu installation checks without producing release-labelled packages.

## Development packages

Run `make package-deb` on native Ubuntu amd64 to build a development `.deb`
under `target/debian/`, together with its SHA-256 checksum. The target runs
the normal locked `make build` first and packages only the core release payload.
It requires the normal Rust/native build prerequisites, Python 3.11 or newer,
`dpkg-dev`, and `binutils`. Rust remains pinned by `rust-toolchain.toml`.

The package version includes the workspace version, Git commit, source-content
hash (including uncommitted files), and a local Debian revision. It does not
claim that development changes shipped in the published workspace version.
`BUILD-INFO.json` records source identity and the build operating system.
The checksum covers the finished `.deb`. For the same source, build environment,
and payload, packaging uses stable timestamps and root ownership.

Install a downloaded package with:

```sh
sudo apt install ./omnivox_VERSION_amd64.deb
omnivox --version
omnivox --engine espeak --list-voices
```

Use `sudo apt remove omnivox` to uninstall it. A newer package can be installed
with the same `apt install ./...deb` command. No service is enabled, no token
is generated, and no user configuration is modified during installation.
Run speech as your ordinary desktop user so it can access the audio session.

The executable and RHVoice helper live under `/usr/lib/omnivox`, with
`/usr/bin/omnivox` as the command. Matching eSpeak data lives under
`/usr/share/omnivox`; relative links preserve the existing adjacent-data
discovery contract. Notices and build identity are under
`/usr/share/doc/omnivox`, and the optional Emacspeak adapter source is under
`/usr/share/emacs/site-lisp/omnivox` (it is not loaded automatically).

The package includes no RHVoice runtime or voices, Flite, RuTTS, Piper,
TGSpeechBox, proprietary runtime, or downloaded voice model. Optional engines
retain their existing separately installed companion boundaries.

Library dependencies are computed from both executables with `dpkg-shlibdeps`.
Build on the oldest Ubuntu release you intend to support and test installation
and synthesis on every claimed release. A package built on Ubuntu 26.04 must
not be assumed compatible with 24.04. This initial target supports amd64 only;
it is not Debian archive or PPA source packaging.

For an intentional packaging revision, use
`python3 tools/package_deb.py --revision 0local2`. This reruns the build as well.

Verify a built package using Docker:

```sh
python3 tools/test_deb_install.py target/debian/omnivox_VERSION_amd64.deb
```

The verifier checks the checksum, payload boundaries, ownership and permissions,
then installs into a disposable `ubuntu:26.04` container. It exercises voice
discovery and non-silent WAV synthesis as an unprivileged user, checks installed
file hashes, reinstalls, upgrades using a container-only packaging fixture, and
purges the package while preserving a sample user configuration file. It mounts
only the candidate and verification tools, so no development runtime data can
mask a missing package payload. It does not install anything on the host.
It enables installation of Omnivox's documentation and notices, which minimal
Ubuntu container images otherwise exclude through their dpkg configuration.
`--image ubuntu:24.04` selects an older compatibility test when appropriate.
These headless checks do not verify playback through a physical audio device.

The default `make package-deb` remains a local development build, even from a
tagged checkout. Run `make deb-package-test release-asset-test` to check the
release guards, expected identity checks, and exact publication asset set.
Release publication continues through the guarded tag workflow and its normal
release review; neither packaging target performs publication.
