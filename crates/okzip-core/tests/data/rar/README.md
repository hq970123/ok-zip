# RAR test fixtures

Real RAR archives (made with RAR/WinRAR) used by `crates/okzip-core/tests/rar.rs`.
Total size is about 65 KB. Nothing here is linked into OK Zip; these files are
only read by the tests.

## `ssokolow/` — CC0 1.0

From [ssokolow/rar-test-files](https://github.com/ssokolow/rar-test-files)
(commit `16b785c2b1b504e99fc307676e5369a26d3ce060`), by Stephan Sokolow, released
into the public domain under CC0 1.0. See `ssokolow/LICENSE.md` and
`ssokolow/LICENSE.cc0`. Copied unchanged from that repository's `build/` folder;
`expected/` is its `sources/` folder (the original files inside the archives).

| File | Format | What it covers |
|---|---|---|
| `testfile.rar3.rar` | RAR 3 (RAR 3.93 for DOS) | single compressed file `testfile.txt` |
| `testfile.rar3.locked.rar` | RAR 3 | locked archive |
| `testfile.rar3.cbr` | RAR 3 | two files (`testfile.jpg`, `testfile.png`) |
| `testfile.rar3.solid.cbr` | RAR 3, solid | two files, solid |
| `testfile.rar5.rar` | RAR 5 (RAR 6.0.2 for Linux) | single file |
| `testfile.rar5.rr.rar` | RAR 5 | with recovery record |
| `testfile.rar5.cbr` | RAR 5 | two files |
| `testfile.rar5.solid.cbr` | RAR 5, solid | two files, solid |

That repository has no encrypted, multi-volume or Unicode-name archives, so
those cases come from the next set.

## `rars/` — Apache License 2.0

Fixtures from the test suite of [bitplane/rars](https://github.com/bitplane/rars)
(crate `rars` 0.12.4, `tests/fixtures/`), Copyright the rars authors, licensed
under the Apache License 2.0 (`rars/LICENSE-APACHE-2.0`). Copied unchanged. Per
that project's fixture notes they were produced with RAR/WinRAR (3.00, 4.20,
5.x/7.x).

| File | Source path in rars | Password | What it covers |
|---|---|---|---|
| `rar4_mixed_visible_names_password.rar` | `rar15_40/encrypted/` | `known-pass` | RAR 4 (WinRAR 4.20), AES per-file encryption, Unicode name `2中文.txt`, one unencrypted member |
| `header_rar420_password.rar` | `rar15_40/encrypted/` | `password` | RAR 4.20 encrypted headers (`-hp`) |
| `solid_simple_rar300.rar` | `rar15_40/rar300/` | – | RAR 3.00 solid |
| `multivol_oldnaming_rar300.rar`, `.r00` | `rar15_40/rar300/` | – | RAR 3.00 multi-volume, old `.rNN` naming |
| `password_aes.rar` | `rar50/` | `password` | RAR 5 AES-256 file encryption |
| `header_encrypted.rar` | `rar50/` | `password` | RAR 5 encrypted headers |
| `solid.rar` | `rar50/` | – | RAR 5 solid |
| `multivol.part1-3.rar` | `rar50/` | – | RAR 5 multi-volume, `.partN.rar` naming |
| `encrypted_multivol.part1-3.rar` | `rar50/` | `password` | RAR 5 encrypted multi-volume |
