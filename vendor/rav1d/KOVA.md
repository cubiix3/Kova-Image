# rav1d 1.1.0 as used by Kova Image

Published source of rav1d 1.1.0 (BSD-2-Clause, see COPYING) with one change:
every `extern "C"` is declared `extern "C-unwind"`, so a panic inside the decoder
can be caught instead of aborting the process. The assembly routines, the build
script and the C-only files are left out. `scripts/vendor-rav1d.py` produces this
directory; comparing it with the published crate shows only that edit
(159 occurrences) and `#![allow(warnings)]` in lib.rs.
