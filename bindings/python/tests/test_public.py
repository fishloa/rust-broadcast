"""Tests against the committed public fixture (no private submodule needed).

`test_smoke.py` skips wholesale when the private tnt capture is absent; these
cover the dvb-si 12 / dvb-t2mi 12 surface on `fixtures/ts/m6-single.ts`.
"""

import os
import pytest
import dvb_si_py as dvb

PUBLIC_FIXTURE = os.path.join(
    os.path.dirname(__file__), "..", "..", "..", "fixtures", "ts", "m6-single.ts"
)
PKT = 188


@pytest.mark.skipif(not os.path.exists(PUBLIC_FIXTURE), reason="fixtures/ts not present")
def test_public_fixture_pat_pmt():
    with open(PUBLIC_FIXTURE, "rb") as f:
        data = f.read()
    dm = dvb.Demux()
    secs = []
    for i in range(0, len(data) - PKT, PKT):
        secs += dm.feed(data[i : i + PKT])
    kinds = {next(iter(s)) for s in secs}
    assert {"patSection", "pmtSection"} <= kinds


def test_t2mi_demux_pid_is_13_bit():
    """The PID is the typed 13-bit `Pid` (dvb-t2mi 12): out-of-range is rejected."""
    dvb.T2miDemux(0x1FFF)
    with pytest.raises(ValueError):
        dvb.T2miDemux(0x2000)
