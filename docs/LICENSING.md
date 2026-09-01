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

## Windows Eloquence and DECtalk helpers

The C# source under [`windows-helpers`](../windows-helpers/README.md) carries
its own Bart Bunting copyright and `GPL-2.0-or-later` notice. The complete GPL
version 2 text is included as
[`windows-helpers/COPYING`](../windows-helpers/COPYING). Those files are not
relicensed by the repository's MIT license.

The helpers are separate 32-bit executables rather than code linked into the
MIT-licensed Rust server. A Windows bundle containing either helper must retain
the applicable GPL notice and license text. Emacsvox stages that text as
`WINDOWS-HELPERS-COPYING` beside its helper executables.

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
real synthesis with a CI-only model that is never uploaded. That exact model
revision is accepted only for CI based on its model card's public-domain
LibriVox and trained-from-scratch declarations; this is not approval to add it
to a release or recommend it to users.

The deterministic `omnivox-VERSION-piper-source.tar.gz` candidate contains the
exact committed Omnivox and libpiper source, every locked Cargo dependency
source, the eSpeak NG and Sonic sources, all four ONNX Runtime binary build
inputs, and the corresponding ONNX Runtime source. Its verifier checks an
exhaustive manifest, the recorded Git tree and input locks, model exclusion,
and offline Cargo resolution. The tag workflow includes this artifact and all
four companions in its draft and verification gates. Release code remains
unsigned and no Piper companion archive is currently published. See the
[Piper release plan](plans/PIPER-RELEASE.md) for the remaining release work.

## Optional RHVoice integration

The MIT-licensed `omnivox-rhvoice-helper` is a separate executable that loads a
user-installed RHVoice C API library at run time. Generic Omnivox payloads may
contain that helper but do not contain the RHVoice library, its language data,
or voice data.

Upstream describes the main RHVoice library as LGPL-2.1-or-later, with combined
build terms affected by optional components, and documents additional or
restrictive terms for some voices. The user's selected runtime and voice terms
continue to apply. See the [RHVoice companion guide](RHVOICE.md) and the licence
files supplied by RHVoice and each installed voice.

## Proprietary engines and other dependencies

Eloquence and DECtalk runtimes and their dictionary or voice data are
user-supplied and are not distributed by Omnivox. Their vendor terms continue
to apply. Public availability of a DECtalk source or binary download does not
replace its restrictive `LICENCE` or establish that a particular user is
authorized to use it. Omnivox also depends on Rust crates and platform
frameworks that retain their own licenses. This component map is not an
exhaustive replacement for their source license files or package metadata.
