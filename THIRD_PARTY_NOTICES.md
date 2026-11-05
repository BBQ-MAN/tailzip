# Third-party notices

tailzip itself is MIT licensed (see [LICENSE](LICENSE)). It is built with many Rust crates
under MIT and/or Apache-2.0 licences (`cargo tree` in `app/` lists them). The components
below need a specific mention.

## UnRAR (RAR extraction)

tailzip opens, tests and extracts RAR archives with the official UnRAR source code
(version 7.1, bundled by the [`unrar_sys`](https://crates.io/crates/unrar_sys) crate and used
through [`unrar`](https://github.com/muja/unrar.rs), MIT OR Apache-2.0). tailzip does not
create RAR archives. UnRAR is not modified. Its licence:

```
 ******    *****   ******   UnRAR - free utility for RAR archives
 **   **  **   **  **   **  ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
 ******   *******  ******    License for use and distribution of
 **   **  **   **  **   **   ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
 **   **  **   **  **   **         FREE portable version
                                   ~~~~~~~~~~~~~~~~~~~~~

      The source code of UnRAR utility is freeware. This means:

   1. All copyrights to RAR and the utility UnRAR are exclusively
      owned by the author - Alexander Roshal.

   2. UnRAR source code may be used in any software to handle
      RAR archives without limitations free of charge, but cannot be
      used to develop RAR (WinRAR) compatible archiver and to
      re-create RAR compression algorithm, which is proprietary.
      Distribution of modified UnRAR source code in separate form
      or as a part of other software is permitted, provided that
      full text of this paragraph, starting from "UnRAR source code"
      words, is included in license, or in documentation if license
      is not available, and in source code comments of resulting package.

   3. The UnRAR utility may be freely distributed. It is allowed
      to distribute UnRAR inside of other software packages.

   4. THE RAR ARCHIVER AND THE UnRAR UTILITY ARE DISTRIBUTED "AS IS".
      NO WARRANTY OF ANY KIND IS EXPRESSED OR IMPLIED.  YOU USE AT
      YOUR OWN RISK. THE AUTHOR WILL NOT BE LIABLE FOR DATA LOSS,
      DAMAGES, LOSS OF PROFITS OR ANY OTHER KIND OF LOSS WHILE USING
      OR MISUSING THIS SOFTWARE.

   5. Installing and using the UnRAR utility signifies acceptance of
      these terms and conditions of the license.

   6. If you don't agree with terms of the license you must remove
      UnRAR files from your storage devices and cease to use the
      utility.

      Thank you for your interest in RAR and UnRAR.


                                            Alexander L. Roshal
```

## ALZ and EGG: unalz, unegg, libazo (MIT)

ALZ and EGG archives are read with these crates, used unmodified as dependencies:

- [unalz](https://github.com/alkegi/unalz) 0.2 — ALZ reader
- [unegg](https://github.com/alkegi/unegg) 0.2 — EGG reader (AES, LEA, solid, split volumes)
- [libazo](https://github.com/alkegi/libazo) 0.1 — AZO decompression, used by unegg

They implement the CC0 format specification at https://github.com/alkegi/docs
(`alz.md`, `egg.md`).

```
MIT License

Copyright (c) 2026 Sungjoon Moon

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Test fixtures (`app/tests/fixtures/`)

| Files | Source | Licence |
|---|---|---|
| `alz/*.alz`, `alz/*.a00`, `alz/*.a01` | [alkegi/unalz `tests/data`](https://github.com/alkegi/unalz/tree/master/tests/data), made by real archiver builds | CC0 1.0 |
| `egg/*.egg` | [alkegi/unegg `tests/data`](https://github.com/alkegi/unegg/tree/master/tests/data), made by real archiver builds | CC0 1.0 |
| `rar4/version.rar`, `rar4/crypted.rar` | test data of the [unrar](https://github.com/muja/unrar.rs) crate (`data/`) | MIT OR Apache-2.0 |
| everything else (`*.zip`, `*.zipx`, `*.7z*`, `rar5*.rar`) | made for tailzip by `make.sh` (7-Zip; RAR used only to create test files) | MIT, like tailzip |

The ALZ/EGG samples use the password `test1234`, `rar4/crypted.rar` uses `unrar`, and the
tailzip-made ones use `Test1234`. Larger upstream samples (for example the 3.5 MB `split.alz`
set) are not committed; an optional test uses them from `app/tests/cache/` when present.
