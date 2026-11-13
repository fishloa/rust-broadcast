"""tools/release_plan.py: levels, needs-attention detection, one-tag-per-push invariant. No network, no git writes."""

import importlib.util
import pathlib
import sys

import pytest

SCRIPT = pathlib.Path(__file__).parents[1] / "release_plan.py"
spec = importlib.util.spec_from_file_location("release_plan", SCRIPT)
rp = importlib.util.module_from_spec(spec)
sys.modules["release_plan"] = rp
spec.loader.exec_module(rp)


def dep(name, kind=None, req="^1"):
    return {"name": name, "kind": kind, "req": req}


def test_dev_dep_edges_only_when_versioned():
    members = {"a", "b", "c", "ext"}
    deps = [dep("a"), dep("b", "dev", "*"), dep("c", "dev", "^0.7.0"), dep("ext2"), dep("a", "build")]
    assert rp.publish_edges(deps, members) == {"a", "c"}


def test_levels_layers_and_satisfied_deps():
    levels = rp.compute_levels({"root": set(), "x": {"root"}, "y": {"root"}, "z": {"x", "y"}, "w": {"already-live"}})
    assert levels == [["root", "w"], ["x", "y"], ["z"]]


def test_levels_cycle_is_an_error():
    with pytest.raises(ValueError):
        rp.compute_levels({"a": {"b"}, "b": {"a"}})


def ws():
    mk = rp.Crate
    return {
        "broadcast-common": mk("broadcast-common", "12.1.0", "broadcast-common"),
        "dvb-si": mk("dvb-si", "14.0.0", "dvb-si", {"broadcast-common"}),
        "dvb-tools": mk("dvb-tools", "14.0.0", "dvb-tools", {"dvb-si"}),
        "dvb-ci": mk("dvb-ci", "0.3.0", "dvb-ci", {"dvb-si"}),
        "mpeg-ts": mk("mpeg-ts", "0.7.0", "mpeg-ts", {"broadcast-common", "mp4-emsg"}),
        "mp4-emsg": mk("mp4-emsg", "0.8.0", "mp4-emsg", {"broadcast-common"}),
    }


def v(w, node, crates):
    c = [w[n] for n in crates]
    return rp.classify(node, c, {}, None, False, True)


def test_lockstep_is_one_node_and_dev_dep_orders_it():
    w = ws()
    verdicts = [
        rp.Verdict("broadcast-common", "broadcast-common-v12.1.0", "12.1.0", "to-publish", crates=("broadcast-common",)),
        rp.Verdict(rp.LOCKSTEP_NODE, "v14.0.0", "14.0.0", "to-publish", crates=("dvb-si", "dvb-tools")),
        rp.Verdict("dvb-ci", "dvb-ci-v0.3.0", "0.3.0", "to-publish", crates=("dvb-ci",)),
        rp.Verdict("mpeg-ts", "mpeg-ts-v0.7.0", "0.7.0", "to-publish", crates=("mpeg-ts",)),
    ]
    nd = rp.node_deps(verdicts, w)
    assert nd[rp.LOCKSTEP_NODE] == {"broadcast-common"}
    assert nd["dvb-ci"] == {rp.LOCKSTEP_NODE}
    assert nd["mpeg-ts"] == {"broadcast-common"}  # mp4-emsg already live: satisfied
    assert rp.compute_levels(nd) == [["broadcast-common"], ["LOCKSTEP", "mpeg-ts"], ["dvb-ci"]]


def cl(live, tagv, changed, empty, version="1.0.0"):
    return rp.classify("c", [rp.Crate("c", version, "c")], {"c": live}, tagv, changed, empty)


def test_classification_table():
    assert cl("1.0.0", "1.0.0", False, True).status == "live"
    assert cl("0.9.0", "0.9.0", False, True).status == "to-publish"
    assert cl(None, None, False, True).status == "to-publish"
    a = cl("1.0.0", "1.0.0", True, True)
    assert a.status == "needs-attention" and "empty" in a.reason
    assert cl("1.0.0", "1.0.0", True, False).status == "needs-attention"
    assert cl("1.0.0", "0.9.0", True, True).status == "live"  # bumped after the tag, live: skip
    assert cl("1.1.0", "1.0.0", False, True).status == "needs-attention"  # manifest behind crates.io
    assert cl("0.9.0", "1.0.0", True, True).status == "needs-attention"  # unbumped + empty changelog


def test_lockstep_disagreeing_versions_need_attention():
    cs = [rp.Crate("dvb-si", "14.0.0", "dvb-si"), rp.Crate("dvb-tools", "13.0.0", "dvb-tools")]
    assert rp.classify(rp.LOCKSTEP_NODE, cs, {}, None, False, True).status == "needs-attention"


def test_unreleased_body():
    assert rp.unreleased_body("# C\n\n## [Unreleased]\n\n## [1.0.0] - x\n- a\n") == ""
    assert "feat" in rp.unreleased_body("## [Unreleased]\n\n### Added\n- feat\n\n## [1.0.0]\n")


def test_live_versions_retries_429_then_raises():
    cio = rp.crates_io()
    calls = []

    def lookup(name):
        calls.append(name)
        if len(calls) < 3:
            raise cio.LookupError_("x: HTTP 429")
        return "1.2.3"

    assert rp.live_versions(["x"], lookup=lookup, sleep=lambda s: None) == {"x": "1.2.3"}

    def always(name):
        raise cio.LookupError_("x: HTTP 429")

    with pytest.raises(cio.LookupError_):
        rp.live_versions(["x"], lookup=always, sleep=lambda s: None)


def pv(crate, ver="1.0.0"):
    return rp.Verdict(crate, f"{crate}-v{ver}", ver, "to-publish", crates=(crate,))


def test_one_tag_per_push_and_level_barrier():
    verdicts = [pv("a"), pv("b"), pv("c"), pv("d")]
    levels = [["a", "b", "c"], ["d"]]
    calls = []

    def runner(tag):
        calls.append(tag)
        return 0, ""

    assert rp.run_levels(verdicts, levels, runner, cap=2, stagger=0, sleep=lambda s: None, out=lambda *_: None) == 0
    assert sorted(calls) == ["a-v1.0.0", "b-v1.0.0", "c-v1.0.0", "d-v1.0.0"]
    assert calls[-1] == "d-v1.0.0"  # level barrier


def test_script_runner_passes_exactly_one_tag_and_never_plan(monkeypatch, tmp_path):
    seen = []

    class R:
        returncode, stdout, stderr = 0, "", ""

    monkeypatch.setattr(rp.subprocess, "run", lambda argv, **k: (seen.append(argv), R())[1])
    run = rp.script_runner(False, ["abc"], log_dir=tmp_path)
    run("a-v1.0.0")
    run("b-v1.0.0")
    for argv in seen:
        tags = [x for x in argv if x.startswith(("a-v", "b-v"))]
        assert len(tags) == 1 and "--plan" not in argv and argv[0] == "bash"
        assert "--allow-tag-commit" in argv and "--dry-run" not in argv
    rp.script_runner(True, [])("a-v1.0.0")
    assert "--dry-run" in seen[-1]


def test_failure_stops_before_next_level_and_reports_429():
    verdicts = [pv("a"), pv("b"), pv("c")]
    started = []

    def runner(tag):
        started.append(tag)
        return (1, "boom") if tag.startswith("a") else (0, "")

    log = []
    rc = rp.run_levels(verdicts, [["a", "b"], ["c"]], runner, stagger=0, sleep=lambda s: None, detect_429=lambda t: True, out=log.append)
    assert rc == 1 and "c-v1.0.0" not in started
    assert any("429" in line for line in log)


def test_manual_stop_exit_2_and_lockstep_gate():
    verdicts = [pv("a"), rp.Verdict(rp.LOCKSTEP_NODE, "v14.0.0", "14.0.0", "to-publish", crates=("dvb-si",)), pv("z")]
    started = []

    def runner(tag):
        started.append(tag)
        return 0, ""

    rc = rp.run_levels(verdicts, [["a", rp.LOCKSTEP_NODE], ["z"]], runner, stagger=0, sleep=lambda s: None, lockstep_is_live=lambda ver: False, out=lambda *_: None)
    assert rc == 2 and started == ["a-v1.0.0"]
    rc = rp.run_levels(verdicts, [["a", rp.LOCKSTEP_NODE], ["z"]], runner, stagger=0, sleep=lambda s: None, lockstep_is_live=lambda ver: True, out=lambda *_: None)
    assert rc == 0 and started[-1] == "z-v1.0.0"
    rc = rp.run_levels([pv("a")], [["a"]], lambda t: (2, "waiting"), stagger=0, sleep=lambda s: None, out=lambda *_: None)
    assert rc == 2
