"""Self-test for tools/differential-sweep/diff.py using synthetic JSONL."""

import importlib.util
import json
import pathlib

SCRIPT = pathlib.Path(__file__).parents[1] / "differential-sweep" / "diff.py"
spec = importlib.util.spec_from_file_location("sweep_diff", SCRIPT)
sd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sd)


def write(path, records):
    path.write_text("\n".join(json.dumps(r) for r in records) + "\n")


def rec(fixture, probe, result):
    return {"fixture": fixture, "probe": probe, "result": result}


def test_identical_outputs_exit_zero(tmp_path, capsys):
    a, b = tmp_path / "a.jsonl", tmp_path / "b.jsonl"
    r = [rec("f.ts", "p", {"n": 1, "xs": [1, 2]})]
    write(a, r)
    write(b, r)
    assert sd.main([str(a), str(b)]) == 0
    assert "UNEXPLAINED=0" in capsys.readouterr().out


def test_difference_is_unexplained_and_fails(tmp_path, capsys):
    a, b = tmp_path / "a.jsonl", tmp_path / "b.jsonl"
    write(a, [rec("f.ts", "transmux.stream", {"tracks": {"1": {"samples": 10}}})])
    write(b, [rec("f.ts", "transmux.stream", {"tracks": {"1": {"samples": 9}}})])
    assert sd.main([str(a), str(b)]) == 1
    out = capsys.readouterr().out
    assert "UNEXPLAINED=1" in out
    assert "tracks.#.samples" in out  # numeric segments are grouped


def test_added_and_removed_keys_and_records(tmp_path):
    base = {("f", "p"): {"a": 1, "gone": 2}, ("only-base", "p"): {"x": 1}}
    head = {("f", "p"): {"a": 1, "new": 3}, ("only-head", "p"): {"x": 1}}
    rows = sd.diff_records(base, head)
    paths = {(r["fixture"], r["path"]) for r in rows}
    assert paths == {
        ("f", "gone"),
        ("f", "new"),
        ("only-base", "<record>"),
        ("only-head", "<record>"),
    }


def test_head_only_keys_ignored():
    base = {("f", "p"): {"a": 1}}
    head = {("f", "p"): {"a": 1, "head_only_identical": 5}}
    assert sd.diff_records(base, head) == []


def test_expected_marks_intended_and_passes(tmp_path, capsys):
    a, b, e = tmp_path / "a.jsonl", tmp_path / "b.jsonl", tmp_path / "e.json"
    write(a, [rec("f.ts", "dvb-tools", {"exit": 0})])
    write(b, [rec("f.ts", "dvb-tools", {"exit": 1})])
    e.write_text(json.dumps([{"probe": "dvb-tools", "path": "exit", "reason": "CHANGELOG BREAKING"}]))
    assert sd.main([str(a), str(b), "--expected", str(e)]) == 0
    assert "INTENDED=1" in capsys.readouterr().out


def test_expected_bug_class_still_fails(tmp_path):
    a, b, e = tmp_path / "a.jsonl", tmp_path / "b.jsonl", tmp_path / "e.json"
    write(a, [rec("f.ts", "p", {"n": 2})])
    write(b, [rec("f.ts", "p", {"n": 1})])
    e.write_text(json.dumps([{"probe": "p", "class": "BUG", "reason": "tracked in #1"}]))
    assert sd.main([str(a), str(b), "--expected", str(e)]) == 1


def test_expected_without_reason_is_rejected(tmp_path):
    a, e = tmp_path / "a.jsonl", tmp_path / "e.json"
    write(a, [rec("f", "p", {})])
    e.write_text(json.dumps([{"probe": "p"}]))
    try:
        sd.main([str(a), str(a), "--expected", str(e)])
    except SystemExit as exc:
        assert exc.code == 2
    else:
        raise AssertionError("expected a usage error")


def test_numeric_segments_group_together():
    rows = [
        dict(fixture="a", probe="p", path="tracks.1.samples", base=1, head=2),
        dict(fixture="b", probe="p", path="tracks.2.samples", base=1, head=2),
    ]
    assert sd.group_key(rows[0]) == sd.group_key(rows[1])
