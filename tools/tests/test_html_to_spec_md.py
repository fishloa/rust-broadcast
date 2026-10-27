"""Self-test for tools/html_to_spec_md.py."""

import importlib.util
import pathlib
import subprocess
import sys

TOOLS = pathlib.Path(__file__).parents[1]
SCRIPT = TOOLS / "html_to_spec_md.py"


def _load():
    spec = importlib.util.spec_from_file_location("html_to_spec_md", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _run(html, tmp_path, *extra):
    src = tmp_path / "in.html"
    src.write_text(html, encoding="utf-8")
    out = tmp_path / "out.md"
    cmd = [sys.executable, str(SCRIPT), "--input", str(src), "--output", str(out), *extra]
    subprocess.run(cmd, check=True)
    return out.read_text(encoding="utf-8")


def test_headings(tmp_path):
    md = _run("<h1>Title</h1><h2>Sub</h2><h3>Deep</h3>", tmp_path)
    assert "# Title" in md
    assert "## Sub" in md
    assert "### Deep" in md


def test_nested_list(tmp_path):
    html = "<ul><li>top<ul><li>nested</li></ul></li></ul>"
    md = _run(html, tmp_path)
    assert "- top" in md
    assert "  - nested" in md


def test_table_colspan(tmp_path):
    html = "<table><tr><td colspan=\"2\">A</td></tr><tr><td>B</td><td>C</td></tr></table>"
    md = _run(html, tmp_path)
    lines = [l for l in md.splitlines() if l.strip().startswith("|")]
    assert len(lines) >= 3
    assert "A" in lines[0]
    assert lines[1].count("---") >= 2


def test_code_block(tmp_path):
    html = "<pre><code class=\"language-swift\">let x = 1</code></pre>"
    md = _run(html, tmp_path)
    assert "```swift" in md
    assert "let x = 1" in md
    assert md.count("```") >= 2


def test_link(tmp_path):
    html = "<p>see <a href=\"https://example.com\">here</a></p>"
    md = _run(html, tmp_path)
    assert "[here](https://example.com)" in md


def test_dropped_nav(tmp_path):
    html = "<nav><a href=\"#x\">navlink</a></nav><p>kept</p>"
    md = _run(html, tmp_path)
    assert "navlink" not in md
    assert "kept" in md


def test_module_convert_function():
    mod = _load()
    md = mod.convert("<h1>H</h1><p>hi</p>", source="src.html", command="cmd", date_str="2020-01-01")
    assert "# H" in md
    assert "hi" in md
    assert "src.html" in md
    assert "2020-01-01" in md
