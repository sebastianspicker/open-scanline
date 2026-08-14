# Third-party notices

## Locked Rust dependency closure

`assets/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md` records the package name,
version, and declared license metadata for every third-party package reported
by `cargo tree --locked --all-features --target all -e normal` when this
bundle was generated. It covers the locked all-feature closure for every
target Cargo reports, so it covers every feature and target variant of an
open-scanline binary built from this locked workspace, rather than only the
packaging host or a no-GUI build. It also reproduces
every locally cached crate file named `LICENSE*`, `COPYING*`, `NOTICE*`, or
`COPYRIGHT*` for that closure. The deterministic
`scripts/generate_rust_dependency_licenses.py --check` command verifies that
the tracked bundle still matches the locked dependency metadata and sources.

The portable archive embeds that file verbatim at
`open-scanline/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md`. The recorded metadata
includes `ring` 0.17.14 (`Apache-2.0 AND ISC`), `rustls-webpki` 0.103.13
(`ISC`), and `webpki-roots` 1.0.9 (`CDLA-Permissive-2.0`), together with the
remaining locked all-feature, all-target dependency closure, including Windows-only
packages. This is a source-metadata record; it does not make a legal claim
beyond the cached crate metadata and files.

The `package` command can archive an arbitrary supplied executable. This
bundle does not identify dependencies in a binary built outside this locked
workspace; its distributor remains responsible for third-party notices for
such an executable.

## Cantarell Regular font

`assets/fonts/Cantarell-Regular.ttf` is copied unchanged from
`sctk-adwaita` version 0.10.1, file `src/title/Cantarell-Regular.ttf`, as
distributed by crates.io. It is embedded only as the glyph carrier for
low-opacity searchable-PDF text. Its SHA-256 is
`d15f25bcdaba2e25f54c32d4e29e68fedeb1b7a1d8fbc83f7c4475916e93b55f`.

The source crate's MIT license does not apply to this font asset. The font's
embedded metadata identifies the following copyright holders, and the asset is
licensed under SIL Open Font License 1.1. It is not covered by this project's
`LICENSE`.

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
