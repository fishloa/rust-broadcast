"""Guard test: dvb-csa's committed circuit file is exactly what its generator emits.

`dvb-csa/src/bitsliced/circuits.rs` opens with "generated code, do not edit".
Hand-editing it silently drifts from `tools/gen_circuits.py`, so the next
regeneration reverts the edit (or breaks the build). This test regenerates into
a scratch crate layout, formats the result the same way the documented
regenerate command does, and compares it byte for byte with the committed file.
"""

import pathlib
import shutil
import subprocess
import sys

import pytest

ROOT = pathlib.Path(__file__).resolve().parents[2]
CRATE = ROOT / "dvb-csa"
GENERATOR = CRATE / "tools" / "gen_circuits.py"
COMMITTED = CRATE / "src" / "bitsliced" / "circuits.rs"


def test_generated_circuits_match_committed(tmp_path: pathlib.Path) -> None:
    rustfmt = shutil.which("rustfmt")
    if rustfmt is None:
        pytest.skip("rustfmt is not on PATH; cannot format the generator's output to compare")

    src = tmp_path / "src"
    (src / "bitsliced").mkdir(parents=True)
    shutil.copyfile(CRATE / "src" / "tables.rs", src / "tables.rs")

    subprocess.run(
        [sys.executable, str(GENERATOR), str(tmp_path)],
        check=True,
        capture_output=True,
    )
    generated = src / "bitsliced" / "circuits.rs"
    subprocess.run(
        [rustfmt, "--edition", "2024", str(generated)],
        check=True,
        capture_output=True,
    )

    assert generated.read_text(encoding="utf-8") == COMMITTED.read_text(encoding="utf-8"), (
        "dvb-csa/src/bitsliced/circuits.rs is not what tools/gen_circuits.py emits; "
        "regenerate it with `python3 tools/gen_circuits.py . && cargo fmt -p dvb-csa`"
    )
