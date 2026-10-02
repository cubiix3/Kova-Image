"""Decode random BC7 blocks with the bcdec reference and write them next to the
blocks, as fixtures for the Rust decoder.

    python scripts/bc7-reference.py path/to/bcdec.h

Needs the MSVC compiler (run from a "x64 Native Tools" prompt). bcdec is only
the oracle for generating the expected pixels; it is not part of the program.
"""
import random
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

root = Path(__file__).resolve().parent.parent
output = root / "tests/fixtures"
bcdec = Path(sys.argv[1]).resolve()
cl = shutil.which("cl")
if not cl:
    raise SystemExit("cl.exe was not found; run from an x64 Native Tools prompt")

PER_MODE = 12
rng = random.Random(7)
blocks = bytearray()
for mode in range(8):
    for _ in range(PER_MODE):
        block = bytearray(rng.getrandbits(8) for _ in range(16))
        # Mode m is signalled by m zero bits followed by a one bit.
        block[0] = (block[0] & ~((1 << (mode + 1)) - 1)) | (1 << mode)
        blocks += block
# A reserved mode (first byte zero) decodes to transparent black.
blocks += bytes(16)

harness = r'''
#define BCDEC_IMPLEMENTATION
#include "bcdec.h"
#include <stdio.h>
int main(int argc, char** argv) {
    FILE* in = fopen(argv[1], "rb");
    FILE* out = fopen(argv[2], "wb");
    unsigned char block[16], pixels[64];
    while (fread(block, 1, 16, in) == 16) {
        bcdec_bc7(block, pixels, 16);
        fwrite(pixels, 1, 64, out);
    }
    fclose(in);
    fclose(out);
    return 0;
}
'''
with tempfile.TemporaryDirectory() as scratch:
    scratch = Path(scratch)
    (scratch / "harness.c").write_text(harness)
    shutil.copy(bcdec, scratch / "bcdec.h")
    subprocess.run([cl, "/nologo", "/O2", "harness.c"], cwd=scratch, check=True,
                   stdout=subprocess.DEVNULL)
    (scratch / "in.bin").write_bytes(blocks)
    subprocess.run([str(scratch / "harness.exe"), "in.bin", "out.bin"], cwd=scratch, check=True)
    (output / "bc7.blocks").write_bytes(blocks)
    (output / "bc7.rgba").write_bytes((scratch / "out.bin").read_bytes())
print(len(blocks) // 16, "blocks")
