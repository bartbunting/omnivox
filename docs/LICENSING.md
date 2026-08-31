# Omnivox Licensing

This document maps the separately licensed parts of the Omnivox source and
binary distributions. It is a project-maintainer summary, not legal advice and
not a substitute for reading the included license texts.

## Omnivox-authored source

Except where a file or component carries another notice, Omnivox-authored
source and documentation in this repository are available under the
[MIT License](../LICENSE). The Rust workspace manifests use the SPDX identifier
`MIT`.

The MIT grant applies to the Omnivox-authored source. It does not replace or
weaken the terms of code, data, models, voices, or runtimes supplied by another
project or vendor.

## Emacspeak adapter

[`elisp/omnivox-voices.el`](../elisp/omnivox-voices.el) carries its own copyright
and GNU General Public License version 2 or later notice. That file is not
relicensed by the repository's MIT license.

## eSpeak NG and release executables

The main Omnivox executable statically incorporates eSpeak NG. The eSpeak NG
project declares its code under
[GPL-3.0-or-later](https://github.com/espeak-ng/espeak-ng#license-information).
The adjacent generated `espeak-ng-data` also comes from the matching eSpeak NG
dependency build.

Because eSpeak NG is statically linked, the release executable is a combined
binary whose distribution must satisfy the applicable GPL-3.0-or-later terms.
The separate Omnivox-authored source remains available under MIT; that
permissive grant does not remove the GPL obligations attached to distribution
of the combined binary.

Supported builds stage `third-party-licenses` beside the executable. Its
`THIRD-PARTY-NOTICES.md` records the locked `espeak-rs-sys` input and the
package contains the eSpeak NG GPL text plus applicable Unicode, NetBSD, and
Sonic notices. Preserve that directory when copying or redistributing a
payload.

GitHub release tags and the packaged `omnivox-Cargo.lock` identify the source
inputs used for an Omnivox release. A redistributor remains responsible for
meeting any corresponding-source and other obligations that apply to its own
method of distribution; a source link or notice directory should not be
assumed to satisfy every distribution scenario by itself.

## Optional Piper integration

The generic binary releases do not contain the Piper helper, Piper native
libraries, or voice models. The optional developer build uses the maintained
GPL-3.0-or-later upstream. Its exact `v1.7.0` `libpiper` source and GPL text are
preserved under
[`third-party/piper1-gpl`](../third-party/piper1-gpl/UPSTREAM.md). The
Omnivox-authored Rust wrapper's MIT declaration does not relicense libpiper,
its native dependency stack, or any model. A distributed helper/native-library
combination must satisfy the applicable GPL and other component terms.

Linux x64, Windows x64, and macOS ARM64/x64 native builds verify their locked
eSpeak NG, Sonic, and ONNX Runtime inputs before building and stage their
notices and provenance in a relocatable companion directory. Their
deterministic archive candidates are verified after relocation and exercise
real synthesis with a CI-only model that is never uploaded. The model's
licensing review remains explicitly deferred, as does the companion's final
corresponding-source review and publishing-workflow integration. See the
[Piper release plan](plans/PIPER-RELEASE.md) for the remaining work. No Piper
companion archive is currently published.

## Proprietary engines and other dependencies

Eloquence and DECtalk runtimes, dictionaries, and voices are user-supplied and
are not distributed by Omnivox. Their vendor terms continue to apply. Omnivox
also depends on Rust crates and platform frameworks that retain their own
licenses. This component map is not an exhaustive replacement for their source
license files or package metadata.
