"""tools/release_deps.py: the semver / index / manifest helpers behind push-release-tags.sh."""

import importlib.util
import pathlib

import pytest

SCRIPT = pathlib.Path(__file__).parents[1] / "release_deps.py"
spec = importlib.util.spec_from_file_location("release_deps", SCRIPT)
rd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rd)


@pytest.mark.parametrize(
    "version,req,ok",
    [
        ("10.0.0", "10.0", True),
        ("10.3.2", "10.0", True),
        ("9.9.9", "10.0", False),
        ("11.0.0", "10.0", False),
        ("0.27.0", "0.27", True),
        ("0.27.9", "0.27", True),
        ("0.28.0", "0.27", False),
        ("0.26.9", "0.27", False),
        ("0.0.3", "0.0.3", True),
        ("0.0.4", "0.0.3", False),
        ("1.2.3", "=1.2.3", True),
        ("1.2.4", "=1.2.3", False),
        ("1.2.9", "~1.2.3", True),
        ("1.3.0", "~1.2.3", False),
        ("1.0.0-rc.1", "1.0", False),  # a pre-release never satisfies a plain requirement
        ("12.0.0", "^12", True),
    ],
)
def test_satisfies(version, req, ok):
    assert rd.satisfies(version, req) is ok


def test_unsupported_requirement_is_an_error_not_a_pass():
    with pytest.raises(ValueError):
        rd.satisfies("1.0.0", ">=1.0, <2")


def test_max_is_semver_not_last_line_and_ignores_yanked():
    body = '{"vers":"0.9.0"}\n{"vers":"10.0.0"}\n{"vers":"9.0.1"}\n{"vers":"11.0.0","yanked":true}\n'
    assert rd.max_version(body) == "10.0.0"
    assert rd.max_version('{"vers":"1.0.0","yanked":true}\n') is None
    assert rd.max_version('{"vers":"1.0.0-rc.1"}\n{"vers":"1.0.0"}\n{"vers":"0.9.9"}\n') == "1.0.0"
    assert rd.max_version('{"vers":"2.0.0-rc.2"}\n{"vers":"2.0.0-rc.10"}\n') == "2.0.0-rc.10"


MANIFEST = """
[package]
name = "me"
version = "1.0.0"

[dependencies]
sib      = { path = "../sib", version = "0.3" }
renamed  = { path = "../x", version = "2.1", package = "realname" }
pathonly = { path = "../p" }
serde    = "1"
self_ref = { path = "../me", version = "1.0", package = "me" }

[build-dependencies]
bsib = { path = "../bsib", version = "4" }

[dev-dependencies]
dsib  = { path = "../dsib", version = "5.0" }
dpath = { path = "../dpath" }

[target.'cfg(unix)'.dependencies]
tsib = { path = "../tsib", version = "6.1", optional = true }
"""


def test_workspace_deps_covers_every_table_and_skips_path_only_and_third_party():
    got = dict(rd.workspace_deps(MANIFEST))
    assert got == {"sib": "0.3", "realname": "2.1", "bsib": "4", "dsib": "5.0", "tsib": "6.1"}


def test_workspace_deps_skip_list_removes_same_lane_crates():
    assert "sib" not in dict(rd.workspace_deps(MANIFEST, {"sib"}))


def test_note_ok():
    assert rd.note_ok("# t\n\nbody\n") == (True, "")
    assert rd.note_ok("   \n")[0] is False
    assert rd.note_ok("---\nstatus: DRAFT\n---\n\n# t\n")[0] is False
    assert rd.note_ok("# t\n\nstatus: DRAFT in prose is fine\n")[0] is True


def test_member_dependency_declared_without_path_is_an_error_not_third_party():
    """A sibling declared with a bare version would otherwise be silently ungated."""
    text = '[package]\nname = "me"\nversion = "1.0.0"\n\n[dependencies]\nsib = "0.3"\n'
    assert rd.workspace_deps(text) == []  # unknown membership: treated as third-party
    with pytest.raises(ValueError, match="workspace member"):
        rd.workspace_deps(text, members={"sib"})
    table = '[package]\nname = "me"\nversion = "1.0.0"\n\n[dev-dependencies]\nsib = { version = "0.3" }\n'
    with pytest.raises(ValueError, match="workspace member"):
        rd.workspace_deps(table, members={"sib"})
    # a real third-party crate, a pathed sibling and an inherited one are all fine
    ok = (
        '[package]\nname = "me"\nversion = "1.0.0"\n\n[dependencies]\nserde = "1"\n'
        'sib = { path = "../sib", version = "0.3" }\ninh = { workspace = true }\n'
    )
    assert rd.workspace_deps(ok, members={"sib", "inh"}) == [("sib", "0.3")]


def test_workspace_members_lists_directory_names():
    assert rd.workspace_members('[workspace]\nmembers = ["a", "x/b", "c/"]\n') == ["a", "b", "c"]
    assert rd.workspace_members("[package]\nname = 'x'\n") == []


def test_cli_deps_failure_is_exit_3_with_err_prefix_and_members_flag():
    import subprocess
    import sys

    def run(stdin, *args):
        return subprocess.run([sys.executable, str(SCRIPT), *args], input=stdin, capture_output=True, text=True)

    bad = run("this is [not toml", "deps")
    assert bad.returncode == 3 and bad.stdout.startswith("ERR ")
    text = '[package]\nname = "me"\nversion = "1.0.0"\n\n[dependencies]\nsib = "0.3"\n'
    r = run(text, "deps", "--members", "sib")
    assert r.returncode == 3 and "workspace member" in r.stdout
    assert run(text, "deps", "--members", "other").returncode == 0
