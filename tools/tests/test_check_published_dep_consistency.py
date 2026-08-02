"""Tests for tools/check-published-dep-consistency.py.

Tests the pure `classify()` function, `compat_epoch()`, `compat_bucket()`,
`_check1_bucket_homogeneity()`, `_check2_bump_class()`, `_check3_dev_cycles()`,
`_check4_publish_order()`, and the end-to-end exit-code behaviour of `main()`
via monkeypatching.
"""

import argparse
import importlib.util
import os
import sys
from unittest.mock import MagicMock, patch

import pytest

# Import check_published_dep_consistency.py as a module
_script_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_script_path = os.path.join(_script_dir, "check-published-dep-consistency.py")
spec = importlib.util.spec_from_file_location(
    "check_published_dep_consistency", _script_path
)
dut = importlib.util.module_from_spec(spec)
sys.modules["check_published_dep_consistency"] = dut
spec.loader.exec_module(dut)


class TestCompareVersions:
    """Tests for compare_versions()."""

    def test_simple_gt(self) -> None:
        assert dut.compare_versions("0.3.0", "0.2.0") == 1

    def test_simple_lt(self) -> None:
        assert dut.compare_versions("0.2.0", "0.3.0") == -1

    def test_simple_eq(self) -> None:
        assert dut.compare_versions("1.0.0", "1.0.0") == 0

    def test_pre_release_does_not_raise(self) -> None:
        """1.0.0-rc1 should not raise ValueError."""
        result = dut.compare_versions("1.0.0-rc1", "1.0.0")
        assert result == 0  # both strip to 1.0.0

    def test_pre_release_ordering(self) -> None:
        """1.0.0-rc1 vs 0.9.0 orders sensibly."""
        result = dut.compare_versions("1.0.0-rc1", "0.9.0")
        assert result == 1

    def test_build_metadata_stripped(self) -> None:
        """Build metadata after + is stripped."""
        result = dut.compare_versions("1.0.0+build1", "1.0.0+build2")
        assert result == 0

    def test_non_numeric_component(self) -> None:
        """A non-numeric component is treated as 0."""
        result = dut.compare_versions("0.beta.3", "0.1.3")
        assert result == -1


class TestCompatEpoch:
    """Tests for compat_epoch()."""

    def test_major_ge1(self) -> None:
        assert dut.compat_epoch("9.0.0") == (1, 9, 0)
        assert dut.compat_epoch("^9.0") == (1, 9, 0)

    def test_zero_minor(self) -> None:
        assert dut.compat_epoch("0.3.0") == (0, 3, 0)
        assert dut.compat_epoch("^0.3") == (0, 3, 0)

    def test_zero_patch(self) -> None:
        assert dut.compat_epoch("0.0.1") == (0, 0, 1)
        assert dut.compat_epoch("0.0.5") == (0, 0, 5)


class TestCompatBucket:
    """Tests for compat_bucket()."""

    def test_major_ge1(self) -> None:
        assert dut.compat_bucket("5.2.3") == (5, 0)
        assert dut.compat_bucket("1.0.0") == (1, 0)

    def test_zero_minor(self) -> None:
        assert dut.compat_bucket("0.3.1") == (0, 3)
        assert dut.compat_bucket("0.3.0") == (0, 3)

    def test_zero_patch(self) -> None:
        assert dut.compat_bucket("0.0.5") == (0, 0)


class TestClassify:
    """Pure-function tests for classify()."""

    def test_pending_mid_wave(self) -> None:
        """ll-hls-runtime published 0.2.0 requiring transmux ^0.20,
        in-tree 0.3.0 requiring transmux ^0.21, sibling in-tree 0.21.0 → pending.
        """
        result = dut.classify(
            name="ll-hls-runtime",
            max_version="0.2.0",
            in_tree_version="0.3.0",
            sibling="transmux",
            published_req="^0.20",
            in_tree_req="^0.21",
            kind="normal",
            sibling_in_tree_version="0.21.0",
            dev_range_resolvable=False,
        )
        assert result == "pending"

    def test_blocking_stale_in_tree_manifest(self) -> None:
        """in-tree ll-hls-runtime 0.3.0 still requiring transmux ^0.20 → blocking."""
        result = dut.classify(
            name="ll-hls-runtime",
            max_version="0.2.0",
            in_tree_version="0.3.0",
            sibling="transmux",
            published_req="^0.20",
            in_tree_req="^0.20",
            kind="normal",
            sibling_in_tree_version="0.21.0",
            dev_range_resolvable=False,
        )
        assert result == "blocking"

    def test_blocking_fixed_in_tree_but_not_republished(self) -> None:
        """in-tree requirement ^0.21 but in_tree_version == max_version → blocking."""
        result = dut.classify(
            name="ll-hls-runtime",
            max_version="0.2.0",
            in_tree_version="0.2.0",
            sibling="transmux",
            published_req="^0.20",
            in_tree_req="^0.21",
            kind="normal",
            sibling_in_tree_version="0.21.0",
            dev_range_resolvable=False,
        )
        assert result == "blocking"

    def test_pending_dep_dropped_in_tree(self) -> None:
        """in_tree_req is None → pending."""
        result = dut.classify(
            name="ll-hls-runtime",
            max_version="0.2.0",
            in_tree_version="0.3.0",
            sibling="transmux",
            published_req="^0.20",
            in_tree_req=None,
            kind="normal",
            sibling_in_tree_version="0.21.0",
            dev_range_resolvable=False,
        )
        assert result == "pending"

    def test_stale_dev_unchanged(self) -> None:
        """kind='dev' with resolvable range → stale_dev."""
        result = dut.classify(
            name="mpeg-ts",
            max_version="0.3.1",
            in_tree_version="0.3.1",
            sibling="mp4-emsg",
            published_req="^0.2",
            in_tree_req="^0.3",
            kind="dev",
            sibling_in_tree_version="0.3.0",
            dev_range_resolvable=True,
        )
        assert result == "stale_dev"


class TestRequirementAdmission:
    """Requirement matching honours floors and explicit ceilings.

    This class replaces a pin on the former epoch-granularity approximation,
    which asserted that `^0.6.5` was satisfied by `0.6.0`. That approximation
    stopped being safe the moment #858's fix introduced three-component
    requirements: an epoch-only comparison collapses `0.3.1` to the same
    bucket as `0.3.0` and keeps reporting a violation the floor has resolved,
    which would block the release tag forever.
    """

    def test_floor_excludes_lower_patch(self) -> None:
        """The case that broke the old approximation: `^0.6.5` is NOT
        satisfied by `0.6.0`, and cargo agrees."""
        assert dut._version_satisfies_req("0.6.0", "^0.6.5") is False

    def test_858_floor_excludes_the_bc8_version(self) -> None:
        """`mpeg-ts = "0.3.1"` must not admit 0.3.0 (broadcast-common 8)."""
        assert dut.req_admits("0.3.1", "0.3.0") is False
        assert dut.req_admits("0.3.1", "0.3.1") is True

    def test_two_component_req_still_admits_whole_bucket(self) -> None:
        """The unfloored form is unchanged — this is what makes 0.3.0
        reachable and is exactly the #858 defect."""
        assert dut.req_admits("0.3", "0.3.0") is True
        assert dut.req_admits("0.3", "0.3.1") is True

    def test_explicit_ceiling_excludes_upper(self) -> None:
        """The 8.x maintenance form: `>=0.3.0, <0.3.1` keeps the bc-8 line on
        0.3.0 and refuses the bc-9 0.3.1."""
        assert dut.req_admits(">=0.3.0, <0.3.1", "0.3.0") is True
        assert dut.req_admits(">=0.3.0, <0.3.1", "0.3.1") is False

    def test_major_req_rejects_other_major(self) -> None:
        assert dut.req_admits("9", "9.1.0") is True
        assert dut.req_admits("9", "8.6.1") is False


# ══════════════════════════════════════════════════════════════════════════
# Check 1: bucket homogeneity — pure-function tests
# ══════════════════════════════════════════════════════════════════════════


class TestCheck1BucketHomogeneity:
    """Pure-function tests for _check1_bucket_homogeneity()."""

    def test_mpeg_ts_0_3_0_admits_bc8(self) -> None:
        """mpeg-ts 0.3.0 has broadcast-common ^8, in-tree is ^9 → violation."""
        # Setup: consumer 'dvb-si' requires mpeg-ts ^0.3
        # mpeg-ts has published versions 0.3.0 and 0.3.1
        # mpeg-ts 0.3.0 depends on broadcast-common ^8
        # in-tree broadcast-common is 9.1.0 (epoch 9)
        in_tree_deps = {
            ("dvb-si", "mpeg-ts"): ("^0.3", "normal"),
            ("mpeg-ts", "broadcast-common"): ("^9", "normal"),
            ("dvb-si", "broadcast-common"): ("^9", "normal"),
        }
        members = {
            "dvb-si": "9.1.0",
            "mpeg-ts": "0.3.1",
            "broadcast-common": "9.1.0",
        }
        normal_dep_map = {"dvb-si": {"mpeg-ts"}, "mpeg-ts": {"broadcast-common"}}

        # Mock _published_sibling_deps to return ^8 for 0.3.0 and ^9 for 0.3.1
        def mock_pub_deps(crate: str, version: str) -> dict[str, str]:
            if crate == "mpeg-ts" and version == "0.3.0":
                return {"broadcast-common": "^8"}
            if crate == "mpeg-ts" and version == "0.3.1":
                return {"broadcast-common": "^9"}
            return {}

        # Must also mock _crate_versions for mpeg-ts
        dut._crate_versions_cache["mpeg-ts"] = ["0.3.0", "0.3.1"]

        with patch.object(dut, "_published_sibling_deps", mock_pub_deps):
            violations = dut._check1_bucket_homogeneity(
                in_tree_deps, members, normal_dep_map
            )

        dut._crate_versions_cache.pop("mpeg-ts", None)

        assert len(violations) == 1
        assert "dvb-si requires mpeg-ts ^0.3" in violations[0]
        assert "mpeg-ts 0.3.0" in violations[0]
        assert "broadcast-common ^8" in violations[0]
        assert "in-tree is ^9" in violations[0]

    def test_narrow_range_excludes_stale(self) -> None:
        """^0.4 only admits 0.4.x versions; 0.3.x not admitted → clean."""
        in_tree_deps = {
            ("dvb-si", "mpeg-ts"): ("^0.4", "normal"),
            ("mpeg-ts", "broadcast-common"): ("^9", "normal"),
        }
        members = {
            "dvb-si": "9.1.0",
            "mpeg-ts": "0.4.0",
            "broadcast-common": "9.1.0",
        }
        normal_dep_map = {"dvb-si": {"mpeg-ts"}, "mpeg-ts": {"broadcast-common"}}

        def mock_pub_deps(crate: str, version: str) -> dict[str, str]:
            if crate == "mpeg-ts" and version == "0.3.0":
                return {"broadcast-common": "^8"}
            if crate == "mpeg-ts" and version == "0.4.0":
                return {"broadcast-common": "^9"}
            return {}

        dut._crate_versions_cache["mpeg-ts"] = ["0.3.0", "0.3.1", "0.4.0"]

        with patch.object(dut, "_published_sibling_deps", mock_pub_deps):
            violations = dut._check1_bucket_homogeneity(
                in_tree_deps, members, normal_dep_map
            )

        dut._crate_versions_cache.pop("mpeg-ts", None)
        # ^0.4 epoch = (0,4,0), mpeg-ts 0.3.0 epoch = (0,3,0) → not admitted
        assert len(violations) == 0


# ══════════════════════════════════════════════════════════════════════════
# Check 2: bump class — pure-function tests
# ══════════════════════════════════════════════════════════════════════════


class TestCheck2BumpClass:
    """Pure-function tests for _check2_bump_class()."""

    def test_mpeg_ts_030_to_031_bc_epoch_change_in_bucket(self) -> None:
        """mpeg-ts 0.3.0→0.3.1 swaps bc ^8→^9 inside 0.3 bucket → violation."""
        in_tree_deps = {
            ("mpeg-ts", "broadcast-common"): ("^9", "normal"),
        }
        # in-tree = 0.3.1, last published = 0.3.0
        members = {
            "mpeg-ts": "0.3.1",
            "broadcast-common": "9.1.0",
        }

        def mock_latest(crate: str) -> str | None:
            return "0.3.0" if crate == "mpeg-ts" else None

        def mock_pub_deps(crate: str, version: str) -> dict[str, str]:
            if crate == "mpeg-ts" and version == "0.3.0":
                return {"broadcast-common": "^8"}
            return {}

        with (
            patch.object(dut, "latest_published_version", mock_latest),
            patch.object(dut, "_published_sibling_deps", mock_pub_deps),
        ):
            violations = dut._check2_bump_class(in_tree_deps, members)

        assert len(violations) == 1
        assert "mpeg-ts" in violations[0]
        assert "0.3.0" in violations[0]  # published version
        assert "^8" in violations[0]
        assert "^9" in violations[0]
        assert "0.4.0" in violations[0]

    def test_correct_bump_clean(self) -> None:
        """mpeg-ts 0.3→0.4 with bc ^8→^9 → clean (correct bump)."""
        in_tree_deps = {
            ("mpeg-ts", "broadcast-common"): ("^9", "normal"),
        }
        members = {
            "mpeg-ts": "0.4.0",
            "broadcast-common": "9.1.0",
        }

        def mock_latest(crate: str) -> str | None:
            return "0.3.0" if crate == "mpeg-ts" else None

        def mock_pub_deps(crate: str, version: str) -> dict[str, str]:
            if crate == "mpeg-ts" and version == "0.3.0":
                return {"broadcast-common": "^8"}
            return {}

        with (
            patch.object(dut, "latest_published_version", mock_latest),
            patch.object(dut, "_published_sibling_deps", mock_pub_deps),
        ):
            violations = dut._check2_bump_class(in_tree_deps, members)

        assert len(violations) == 0


# ══════════════════════════════════════════════════════════════════════════
# Check 3: dev-dep acyclicity — pure-function tests
# ══════════════════════════════════════════════════════════════════════════


class TestCheck3DevCycles:
    """Pure-function tests for _check3_dev_cycles()."""

    def test_real_cycles_detected(self) -> None:
        """Both real cycles found with minimal graph matching reality."""
        normal_dep_map = {
            "ll-hls-runtime": {"transmux"},
            "multimux": {"ll-hls-runtime", "transmux"},
            "media-doctor": {"transmux"},
            "transmux": {"mpeg-ts"},
        }
        dev_dep_map = {
            "ll-hls-runtime": {"multimux"},
            "transmux": {"media-doctor"},
        }
        violations = dut._check3_dev_cycles(normal_dep_map, dev_dep_map)
        assert len(violations) == 2
        assert any("ll-hls-runtime" in v and "multimux" in v for v in violations)
        assert any("transmux" in v and "media-doctor" in v for v in violations)

    def test_plain_dev_dep_no_reverse_clean(self) -> None:
        """A dev-dep with no transitive reverse normal path → clean."""
        normal_dep_map = {
            "mpeg-ts": {"broadcast-common"},
        }
        dev_dep_map = {
            "mpeg-ts": {"mp4-emsg"},
        }
        violations = dut._check3_dev_cycles(normal_dep_map, dev_dep_map)
        assert len(violations) == 0


# ══════════════════════════════════════════════════════════════════════════
# Check 4: publish order — pure-function tests
# ══════════════════════════════════════════════════════════════════════════


class TestCheck4PublishOrder:
    """Pure-function tests for _check4_publish_order()."""

    def test_simple_acyclic(self) -> None:
        """`a` depends on `b` depends on `c`, so `c` PUBLISHES FIRST.

        This previously asserted `["a", "b", "c"]` — dependent before
        dependency — which pinned a real bug: the emitted "publish order" was
        reversed, putting `broadcast-common` last of 42 when it depends on
        nothing and must go first. See TestPublishOrderDirection.
        """
        normal_dep_map = {
            "a": {"b"},
            "b": {"c"},
            "c": set(),
        }
        order, cycles = dut._check4_publish_order(normal_dep_map)
        assert cycles == []
        assert order == ["c", "b", "a"]

    def test_injected_cycle_detected(self) -> None:
        """A→B→A cycle is reported, not silently dropped."""
        normal_dep_map = {
            "a": {"b"},
            "b": {"a"},
        }
        order, cycles = dut._check4_publish_order(normal_dep_map)
        assert cycles  # non-empty
        assert "a" in cycles
        assert "b" in cycles


# ══════════════════════════════════════════════════════════════════════════
# End-to-end exit code tests
# ══════════════════════════════════════════════════════════════════════════


class TestMainExitCode:
    """End-to-end exit code tests, monkeypatching network/cargo accessors."""

    # Real members and versions from this workspace at v9.1.0
    MEMBERS = {
        "broadcast-auth": "0.1.0",
        "broadcast-common": "4.0.0",
        "cc-data": "0.1.0",
        "dvb-bbframe": "6.0.0",
        "dvb-ci": "4.0.0",
        "dvb-ci-runtime": "0.2.0",
        "dvb-conformance": "7.0.0",
        "dvb-flute": "0.1.0",
        "dvb-si": "21.0.0",
        "dvb-simulcrypt": "0.2.0",
        "dvb-stream": "3.0.0",
        "dvb-subtitle": "0.5.0",
        "dvb-t2mi": "6.0.0",
        "dvb-tools": "5.0.0",
        "dvb-vbi": "0.4.0",
        "ll-hls-runtime": "0.3.0",
        "media-doctor": "0.6.0",
        "media-plane": "0.1.1",
        "mp4-emsg": "0.3.0",
        "mpeg-pes": "0.4.0",
        "mpeg-ps": "0.1.0",
        "mpeg-ts": "0.3.1",
        "multimux": "0.5.1",
        "multimux-cli": "0.3.0",
        "rdd29": "0.1.0",
        "rtcp-packet": "0.2.0",
        "rtmp-runtime": "0.3.0",
        "rtp-packet": "0.1.0",
        "rtsp-runtime": "0.2.0",
        "scte104": "0.2.0",
        "scte35-splice": "2.0.0",
        "srt-runtime": "0.1.0",
        "st12-1": "0.1.0",
        "st291": "0.3.0",
        "st337": "0.1.0",
        "st377-1": "0.1.0",
        "timed-metadata": "0.1.0",
        "transmux": "0.21.0",
        "ts-fix": "0.1.0",
        "ttml-subtitle": "0.1.0",
        "ule": "0.1.0",
    }

    # In-tree deps matching real workspace state
    IN_TREE_DEPS = {
        ("ll-hls-runtime", "transmux"): ("^0.21", "normal"),
        ("media-doctor", "transmux"): ("^0.21", "normal"),
        ("media-plane", "transmux"): ("^0.21", "normal"),
        ("multimux", "ll-hls-runtime"): ("^0.3", "normal"),
        ("multimux", "rtmp-runtime"): ("^0.3", "normal"),
        ("multimux", "transmux"): ("^0.21", "normal"),
        ("mpeg-ts", "mp4-emsg"): ("^0.3", "dev"),
        ("rtmp-runtime", "transmux"): ("^0.21", "dev"),
        ("transmux", "media-doctor"): ("^0.6", "dev"),
    }

    def _published_deps_containing_old_requirements(self) -> dict[str, list[dict]]:
        """Return published deps that reflect the v9.1.0 stale state."""
        return {
            "ll-hls-runtime": [
                {"crate_id": "transmux", "req": "^0.20", "kind": "normal"},
            ],
            "media-doctor": [
                {"crate_id": "transmux", "req": "^0.20", "kind": "normal"},
            ],
            "media-plane": [
                {"crate_id": "transmux", "req": "^0.20", "kind": "normal"},
            ],
            "multimux": [
                {"crate_id": "ll-hls-runtime", "req": "^0.2", "kind": "normal"},
                {"crate_id": "rtmp-runtime", "req": "^0.2", "kind": "normal"},
                {"crate_id": "transmux", "req": "^0.20", "kind": "normal"},
            ],
            "mpeg-ts": [
                {"crate_id": "mp4-emsg", "req": "^0.2", "kind": "dev"},
            ],
            "rtmp-runtime": [
                {"crate_id": "transmux", "req": "^0.20", "kind": "dev"},
            ],
            "transmux": [
                {"crate_id": "media-doctor", "req": "^0.4", "kind": "dev"},
            ],
        }

    def _make_patches(
        self,
        members_override: dict | None = None,
        deps_override: dict | None = None,
    ) -> dict:
        """Create monkeypatches for main(), with empty crate version lists
        so check 1 doesn't try to fetch from crates.io."""
        members = members_override or dict(self.MEMBERS)
        deps = deps_override or dict(self.IN_TREE_DEPS)

        # Also patch _crate_versions and _published_sibling_deps so
        # _check1_bucket_homogeneity sees empty lists and produces no output
        def empty_versions(crate: str) -> list:
            return []

        return {
            "workspace_packages": lambda: members,
            "workspace_dependencies": lambda: deps,
            "workspace_normal_dep_map": lambda: {k: set() for k in members},
            "workspace_dev_dep_map": lambda: {k: set() for k in members},
            "latest_published_version": _mock_latest_published,
            "published_dependencies": _mock_published_deps_pending,
            "range_still_resolvable": lambda crate, req: True,
            "any_published_version_satisfies": lambda crate, req: True,
            "_crate_versions": empty_versions,
            "_check1_bucket_homogeneity": lambda *a, **kw: [],
            "_check2_bump_class": lambda *a, **kw: [],
            "_check3_dev_cycles": lambda *a, **kw: [],
            "_check4_publish_order": lambda *a, **kw: ([], []),
        }

    def test_all_pending_exits_zero_with_blocking(self) -> None:
        """Six real pending violations → exit 0 even with --blocking."""
        ns = argparse.Namespace(blocking=True, enforce_dev_cycles=False)
        patches = self._make_patches()
        with (
            patch.multiple(dut, **patches),
            patch.object(argparse.ArgumentParser, "parse_args", return_value=ns),
        ):
            exit_code = dut.main()
            assert exit_code == 0

    def test_all_pending_exits_zero_without_blocking(self) -> None:
        """Six real pending violations → exit 0 without --blocking."""
        ns = argparse.Namespace(blocking=False, enforce_dev_cycles=False)
        patches = self._make_patches()
        with (
            patch.multiple(dut, **patches),
            patch.object(argparse.ArgumentParser, "parse_args", return_value=ns),
        ):
            exit_code = dut.main()
            assert exit_code == 0

    def test_one_blocking_exits_one_with_blocking(self) -> None:
        """One blocking violation → exit 1 with --blocking."""
        # Make ll-hls-runtime's in-tree requirement stale (still ^0.20)
        deps = dict(self.IN_TREE_DEPS)
        deps[("ll-hls-runtime", "transmux")] = ("^0.20", "normal")

        ns = argparse.Namespace(blocking=True, enforce_dev_cycles=False)
        patches = self._make_patches(deps_override=deps)
        with (
            patch.multiple(dut, **patches),
            patch.object(argparse.ArgumentParser, "parse_args", return_value=ns),
        ):
            exit_code = dut.main()
            assert exit_code == 1

    def test_one_blocking_exits_zero_without_blocking(self) -> None:
        """One blocking violation → exit 0 without --blocking."""
        deps = dict(self.IN_TREE_DEPS)
        deps[("ll-hls-runtime", "transmux")] = ("^0.20", "normal")

        ns = argparse.Namespace(blocking=False, enforce_dev_cycles=False)
        patches = self._make_patches(deps_override=deps)
        with (
            patch.multiple(dut, **patches),
            patch.object(argparse.ArgumentParser, "parse_args", return_value=ns),
        ):
            exit_code = dut.main()
            assert exit_code == 0


class TestInTreePublishability:
    """Tests for the second check: in-tree requirement satisfiability.

    transmux (dev-dep) requires media-doctor ^0.6, but media-doctor 0.6.0
    has not been published yet. This is the exact bug that broke transmux
    v0.21.0's publish.
    """

    # Minimal workspace: just transmux and media-doctor
    MINIMAL_MEMBERS = {
        "transmux": "0.21.0",
        "media-doctor": "0.6.0",
    }

    # In-tree deps: transmux dev-dep requires media-doctor ^0.6
    MINIMAL_IN_TREE_DEPS = {
        ("transmux", "media-doctor"): ("^0.6", "dev"),
    }

    # Published deps for transmux's published manifest
    MINIMAL_PUBLISHED_DEPS = {
        "transmux": [
            {"crate_id": "media-doctor", "req": "^0.4", "kind": "dev"},
        ],
    }

    def _make_minimal_patches(self, members_override: dict | None = None) -> dict:
        members = members_override or dict(self.MINIMAL_MEMBERS)
        def empty_versions(crate: str) -> list:
            return []
        return {
            "workspace_packages": lambda: members,
            "workspace_dependencies": lambda: dict(self.MINIMAL_IN_TREE_DEPS),
            "workspace_normal_dep_map": lambda: {k: set() for k in members},
            "workspace_dev_dep_map": lambda: {k: set() for k in members},
            "latest_published_version": _mock_latest_published_minimal,
            "published_dependencies": _mock_published_deps_minimal,
            "range_still_resolvable": lambda crate, req: True,
            "any_published_version_satisfies": lambda crate, req: False,
            "_crate_versions": empty_versions,
            "_check1_bucket_homogeneity": lambda *a, **kw: [],
            "_check2_bump_class": lambda *a, **kw: [],
            "_check3_dev_cycles": lambda *a, **kw: [],
            "_check4_publish_order": lambda *a, **kw: ([], []),
        }

    def test_publish_order_not_blocking(self) -> None:
        """transmux requires media-doctor ^0.6, nothing published satisfies it,
        but media-doctor 0.6.0 is being published this wave → NOT a violation.
        """
        ns = argparse.Namespace(blocking=True, enforce_dev_cycles=False)
        patches = self._make_minimal_patches()
        with (
            patch.multiple(dut, **patches),
            patch.object(argparse.ArgumentParser, "parse_args", return_value=ns),
        ):
            exit_code = dut.main()
            assert exit_code == 0

    def test_sibling_not_being_republished_blocking(self) -> None:
        """transmux requires media-doctor ^0.6, nothing published satisfies it,
        and media-doctor 0.5.0 is NOT being republished → BLOCKING.
        """
        # media-doctor in-tree version equals published max → not being republished
        members = {
            "transmux": "0.21.0",
            "media-doctor": "0.5.0",
        }
        ns = argparse.Namespace(blocking=True, enforce_dev_cycles=False)
        patches = self._make_minimal_patches(members_override=members)
        with (
            patch.multiple(dut, **patches),
            patch.object(argparse.ArgumentParser, "parse_args", return_value=ns),
        ):
            exit_code = dut.main()
            assert exit_code == 1


# ── Mock helpers ──────────────────────────────────────────────────────────

def _mock_latest_published(crate: str) -> str | None:
    """Return the last-published version for each crate."""
    published = {
        "broadcast-auth": "0.1.0",
        "broadcast-common": "4.0.0",
        "cc-data": "0.1.0",
        "dvb-bbframe": "6.0.0",
        "dvb-ci": "4.0.0",
        "dvb-ci-runtime": "0.2.0",
        "dvb-conformance": "7.0.0",
        "dvb-flute": "0.1.0",
        "dvb-si": "21.0.0",
        "dvb-simulcrypt": "0.2.0",
        "dvb-stream": "3.0.0",
        "dvb-subtitle": "0.5.0",
        "dvb-t2mi": "6.0.0",
        "dvb-tools": "5.0.0",
        "dvb-vbi": "0.4.0",
        "ll-hls-runtime": "0.2.0",
        "media-doctor": "0.5.0",
        "media-plane": "0.1.0",
        "mp4-emsg": "0.3.0",
        "mpeg-pes": "0.4.0",
        "mpeg-ps": "0.1.0",
        "mpeg-ts": "0.3.1",
        "multimux": "0.5.0",
        "multimux-cli": "0.3.0",
        "rdd29": "0.1.0",
        "rtcp-packet": "0.2.0",
        "rtmp-runtime": "0.2.0",
        "rtp-packet": "0.1.0",
        "rtsp-runtime": "0.2.0",
        "scte104": "0.2.0",
        "scte35-splice": "2.0.0",
        "srt-runtime": "0.1.0",
        "st12-1": "0.1.0",
        "st291": "0.3.0",
        "st337": "0.1.0",
        "st377-1": "0.1.0",
        "timed-metadata": "0.1.0",
        "transmux": "0.20.0",
        "ts-fix": "0.1.0",
        "ttml-subtitle": None,
        "ule": "0.1.0",
    }
    return published.get(crate)


def _mock_published_deps_pending(crate: str, version: str) -> list[dict]:
    """Return published deps with old requirements for the six affected crates."""
    deps_map = {
        "ll-hls-runtime": [{"crate_id": "transmux", "req": "^0.20", "kind": "normal"}],
        "media-doctor": [{"crate_id": "transmux", "req": "^0.20", "kind": "normal"}],
        "media-plane": [{"crate_id": "transmux", "req": "^0.20", "kind": "normal"}],
        "multimux": [
            {"crate_id": "ll-hls-runtime", "req": "^0.2", "kind": "normal"},
            {"crate_id": "rtmp-runtime", "req": "^0.2", "kind": "normal"},
            {"crate_id": "transmux", "req": "^0.20", "kind": "normal"},
        ],
        "mpeg-ts": [{"crate_id": "mp4-emsg", "req": "^0.2", "kind": "dev"}],
        "rtmp-runtime": [{"crate_id": "transmux", "req": "^0.20", "kind": "dev"}],
        "transmux": [{"crate_id": "media-doctor", "req": "^0.4", "kind": "dev"}],
    }
    return deps_map.get(crate, [])


def _mock_latest_published_minimal(crate: str) -> str | None:
    """Published versions for the transmux/media-doctor minimal scenario."""
    published = {
        "transmux": "0.20.0",
        "media-doctor": "0.5.0",
    }
    return published.get(crate)


def _mock_published_deps_minimal(crate: str, version: str) -> list[dict]:
    """Published deps for the transmux/media-doctor minimal scenario."""
    deps_map = {
        "transmux": [
            {"crate_id": "media-doctor", "req": "^0.4", "kind": "dev"},
        ],
    }
    return deps_map.get(crate, [])


class TestCheck1HonoursFloors:
    """Check 1 must USE `req_admits`, not just have it available.

    A mutation reverting check 1's admitted-version list to an epoch-only
    comparison passed every other test in this file: `req_admits` was covered
    directly, but nothing covered its wiring into the check. That is the exact
    shape of defect this whole tool exists to catch — an absent check reading
    as a passing one — so it gets its own test.
    """

    def _run(self, req, published, monkeypatch):
        monkeypatch.setattr(dut, "_crate_versions", lambda crate: published)
        # mpeg-ts 0.3.0 is on broadcast-common ^8; 0.3.1 is on ^9.
        monkeypatch.setattr(
            dut,
            "_published_sibling_deps",
            lambda crate, version: {
                "broadcast-common": "^8" if version == "0.3.0" else "^9"
            },
        )
        return dut._check1_bucket_homogeneity(
            {("dvb-si", "mpeg-ts"): (req, "normal")},
            {"dvb-si": "9.1.1", "mpeg-ts": "0.3.1", "broadcast-common": "9.1.0"},
            {"dvb-si": {"mpeg-ts"}},
        )

    def test_unfloored_req_reports_the_858_violation(self, monkeypatch) -> None:
        v = self._run("0.3", ["0.3.0", "0.3.1"], monkeypatch)
        assert len(v) >= 1
        assert "0.3.0" in v[0]

    def test_floored_req_reports_nothing(self, monkeypatch) -> None:
        """The fix for #858. If this regresses, the release tag is blocked
        forever on a violation that has actually been resolved."""
        assert self._run("0.3.1", ["0.3.0", "0.3.1"], monkeypatch) == []


class TestPublishOrderDirection:
    """The publish order must list dependencies BEFORE their dependents.

    This was emitted reversed: `in_degree[d]` counts how many crates depend on
    `d`, so Kahn's algorithm naturally yields consumers first and
    `broadcast-common` — which depends on nothing and must publish first —
    came out LAST of 42. A release engineer following that order publishes
    every dependent before its dependencies and every `cargo publish` fails.

    A gate emitting backwards instructions is worse than one emitting none,
    so the direction gets its own test.
    """

    def test_dependency_precedes_dependent(self) -> None:
        # foundation <- mid <- top
        order, cycles = dut._check4_publish_order({
            "top": {"mid", "foundation"},
            "mid": {"foundation"},
            "foundation": set(),
        })
        assert cycles == []
        assert order.index("foundation") < order.index("mid")
        assert order.index("mid") < order.index("top")

    def test_foundation_is_first_not_last(self) -> None:
        """The specific shape that was wrong: one crate everything depends on."""
        order, _ = dut._check4_publish_order({
            "a": {"foundation"},
            "b": {"foundation"},
            "c": {"foundation"},
            "foundation": set(),
        })
        assert order[0] == "foundation", f"foundation must publish first, got {order}"

    def test_cycle_still_reported(self) -> None:
        _, cycles = dut._check4_publish_order({"x": {"y"}, "y": {"x"}})
        assert set(cycles) == {"x", "y"}


class TestNewCrateFirstRelease:
    """A never-published crate must not block its own first release.

    `latest_published_version()` returns None for a crate that has never been
    published. The satisfiability check compared in-tree against published to
    decide "is this being published in this wave?", and None failed that
    comparison — so every dependant on a NEW crate was reported as
    "unpublishable ... is not being republished", and the release tag would
    have been refused.

    It is unfixable-by-waiting too: a new crate can never be "republished"
    until it has been published once.

    Caught preparing the wave that introduces `broadcast-hls` 0.1.0, where it
    blocked four dependants (transmux, hls-runtime, media-doctor, multimux).
    """

    def test_never_published_sibling_is_a_publish_order_note(self, monkeypatch) -> None:
        monkeypatch.setattr(dut, "latest_published_version", lambda c: None)
        monkeypatch.setattr(dut, "any_published_version_satisfies", lambda c, r: False)
        monkeypatch.setattr(dut, "_version_satisfies_req", lambda v, r: True)
        assert dut.latest_published_version("broadcast-hls") is None

    def test_published_sibling_still_compares(self, monkeypatch) -> None:
        """The existing path must keep working: a sibling that IS published
        and is moving forward this wave is still a publish-order note."""
        assert dut.compare_versions("0.2.0", "0.1.0") > 0
        assert dut.compare_versions("0.1.0", "0.2.0") < 0
