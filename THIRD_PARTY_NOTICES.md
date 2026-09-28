# Third-party notices

## Locked Rust dependency closure

[`assets/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md`](assets/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md)
records the package name, version, and declared license for every third-party
package in the locked all-feature build. It is generated from
`cargo tree --locked --all-features --target all -e normal`, so it covers every
feature and target variant Cargo reports — including Windows-only packages — not
only the packaging host or a no-GUI build. It also reproduces every cached crate
file named `LICENSE*`, `COPYING*`, `NOTICE*`, or `COPYRIGHT*`. Run
`scripts/generate_rust_dependency_licenses.py --check` to confirm that the tracked
bundle still matches the locked dependency metadata and sources.

The portable archive embeds the file verbatim at
`open-scanline/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md`. It records, for
example, `ring` 0.17.14 (`Apache-2.0 AND ISC`), `rustls-webpki` 0.103.13 (`ISC`),
and `webpki-roots` 1.0.9 (`CDLA-Permissive-2.0`), along with the rest of the
locked all-feature, all-target closure. This is a source-metadata record: it
makes no legal claim beyond the cached crate metadata and files.

The `package` command can also archive an arbitrary supplied executable. This
bundle does not identify dependencies in a binary built outside the locked
workspace, so the distributor of such a binary remains responsible for its
third-party notices.

## Atkinson Hyperlegible Next and Mono fonts

The desktop interface embeds three static font instances:

| File | Source | SHA-256 |
| --- | --- | --- |
| `assets/fonts/AtkinsonHyperlegibleNext-Regular.ttf` | `AtkinsonHyperlegibleNext[wght].ttf`, instanced at `wght=400` | `d8ebd46b368a6dad973a391ec526dfe73a72c81ac8c68fba3414ee84065fd7b3` |
| `assets/fonts/AtkinsonHyperlegibleNext-Bold.ttf` | `AtkinsonHyperlegibleNext[wght].ttf`, instanced at `wght=700` | `0d33da80febdefb7e57b7a3e19c164568c5bfaa7d8624f405ca9d240134bd092` |
| `assets/fonts/AtkinsonHyperlegibleMono-Regular.ttf` | `AtkinsonHyperlegibleMono[wght].ttf`, instanced at `wght=400` | `28f2ed5f9f429a80fce625e90249dbcc588fc2349aa1470980cf05296cfa79bf` |

The variable sources come from the `ofl/atkinsonhyperlegiblenext/` and
`ofl/atkinsonhyperlegiblemono/` directories of the
[google/fonts](https://github.com/google/fonts) repository. The static instances
were produced with `fonttools varLib.instancer --static --update-name-table`;
glyph outlines are otherwise unchanged. The GitHub Pages site serves WOFF2
conversions of the same instances from `docs/assets/fonts/`.

These fonts are licensed under the SIL Open Font License 1.1, whose full text is
reproduced in the Cantarell section below. They are not covered by this project's
[`LICENSE`](LICENSE).

```text
Copyright 2020-2024 The Atkinson Hyperlegible Next Project Authors (https://github.com/googlefonts/atkinson-hyperlegible-next)
Copyright 2020-2024 The Atkinson Hyperlegible Mono Project Authors (https://github.com/googlefonts/atkinson-hyperlegible-next-mono)
```

## Cantarell Regular font

`assets/fonts/Cantarell-Regular.ttf` is copied unchanged from `sctk-adwaita`
0.10.1, file `src/title/Cantarell-Regular.ttf`, as distributed by crates.io. It is
embedded only as the glyph carrier for low-opacity searchable-PDF text. Its
SHA-256 is
`d15f25bcdaba2e25f54c32d4e29e68fedeb1b7a1d8fbc83f7c4475916e93b55f`.

The source crate's MIT license does not apply to this font asset. The font's
embedded metadata identifies the copyright holders below, and the asset is
licensed under the SIL Open Font License 1.1. It is not covered by this project's
[`LICENSE`](LICENSE).

```text
Copyright (c) 2009-2011, Understanding Limited (dave@understandinglimited.com),
Copyright (c) 2010-2011, Jakub Steiner (jimmac@gmail.com).

SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007

PREAMBLE
The goals of the Open Font License (OFL) are to stimulate worldwide development
of collaborative font projects, to support the font creation efforts of
academic and linguistic communities, and to provide a free and open framework
in which fonts may be shared and improved in partnership with others.

The OFL allows the licensed fonts to be used, studied, modified and
redistributed freely as long as they are not sold by themselves. The fonts,
including any derivative works, can be bundled, embedded, redistributed and/or
sold with any software provided that any reserved names are not used by
derivative works. The fonts and derivatives, however, cannot be released under
any other type of license. The requirement for fonts to remain under this
license does not apply to any document created using the fonts or their
derivatives.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright Holder(s)
under this license and clearly marked as such. This may include source files,
build scripts and documentation.

"Reserved Font Name" refers to any names specified as such after the copyright
statement(s).

"Original Version" refers to the collection of Font Software components as
distributed by the Copyright Holder(s).

"Modified Version" refers to any derivative made by adding to, deleting, or
substituting -- in part or in whole -- any of the components of the Original
Version, by changing formats or by porting the Font Software to a new
environment.

"Author" refers to any designer, engineer, programmer, technical writer or
other person who contributed to the Font Software.

PERMISSION & CONDITIONS
Permission is hereby granted, free of charge, to any person obtaining a copy of
the Font Software, to use, study, copy, merge, embed, modify, redistribute,
and sell modified and unmodified copies of the Font Software, subject to the
following conditions:

1) Neither the Font Software nor any of its individual components, in Original
or Modified Versions, may be sold by itself.

2) Original or Modified Versions of the Font Software may be bundled,
redistributed and/or sold with any software, provided that each copy contains
the above copyright notice and this license. These can be included either as
stand-alone text files, human-readable headers or in the appropriate
machine-readable metadata fields within text or binary files as long as those
fields can be easily viewed by the user.

3) No Modified Version of the Font Software may use the Reserved Font Name(s)
unless explicit written permission is granted by the corresponding Copyright
Holder. This restriction only applies to the primary font name as presented to
the users.

4) The name(s) of the Copyright Holder(s) or the Author(s) of the Font Software
shall not be used to promote, endorse or advertise any Modified Version, except
to acknowledge the contribution(s) of the Copyright Holder(s) and the Author(s)
or with their explicit written permission.

5) The Font Software, modified or unmodified, in part or in whole, must be
distributed entirely under this license, and must not be distributed under any
other license. The requirement for fonts to remain under this license does not
apply to any document created using the Font Software.

TERMINATION
This license becomes null and void if any of the above conditions are not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF COPYRIGHT, PATENT,
TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE COPYRIGHT HOLDER BE LIABLE FOR
ANY CLAIM, DAMAGES OR OTHER LIABILITY, INCLUDING ANY GENERAL, SPECIAL,
INDIRECT, INCIDENTAL, OR CONSEQUENTIAL DAMAGES, WHETHER IN AN ACTION OF
CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF THE USE OR INABILITY TO USE
THE FONT SOFTWARE OR FROM OTHER DEALINGS IN THE FONT SOFTWARE.
```
