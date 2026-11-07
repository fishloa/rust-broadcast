"""crates.io rejects a publish whose keywords or categories break its limits, and only at
upload time, mid-wave (rtp-packet-v0.6.0 failed on six keywords). Pin the limits here."""

import pathlib
import re
import tomllib

import pytest

REPO = pathlib.Path(__file__).parents[2]
MAX_KEYWORDS = 5
MAX_CATEGORIES = 5
MAX_KEYWORD_LEN = 20
KEYWORD = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]*$")

MANIFESTS = sorted(
    p for p in REPO.glob("*/Cargo.toml")
    if tomllib.loads(p.read_text()).get("package", {}).get("publish", True) is not False
)


@pytest.mark.parametrize("manifest", MANIFESTS, ids=lambda p: p.parent.name)
def test_crates_io_metadata_limits(manifest):
    pkg = tomllib.loads(manifest.read_text())["package"]
    keywords = pkg.get("keywords", [])
    categories = pkg.get("categories", [])
    assert isinstance(keywords, list) and isinstance(categories, list)
    assert len(keywords) <= MAX_KEYWORDS, f"{len(keywords)} keywords: {keywords}"
    assert len(categories) <= MAX_CATEGORIES, f"{len(categories)} categories: {categories}"
    for k in keywords:
        assert len(k) <= MAX_KEYWORD_LEN and KEYWORD.match(k), f"invalid keyword {k!r}"
