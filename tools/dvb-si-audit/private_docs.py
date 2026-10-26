"""Where the dvb spec markdown lives: the private companion repo, not this tree."""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PRIVATE_DVB = ROOT / "private" / "specs-verified" / "dvb"


def docs_dir(crate: str) -> Path:
    if not PRIVATE_DVB.is_dir():
        sys.exit(
            f"{PRIVATE_DVB} not found: the spec transcription markdown lives only in the private "
            "companion repo (maintainers: `git submodule update --init private`)."
        )
    return PRIVATE_DVB / crate
