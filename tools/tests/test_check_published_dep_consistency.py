"""Tests for tools/check-published-dep-consistency.py.

Tests the pure `classify()` function and the end-to-end exit-code behaviour
of `main()` via monkeypatching.
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


class TestEpochApproximation:
    """Pins the documented epoch-granularity limitation."""

    def test_epoch_granularity_is_a_known_limitation(self) -> None:
        """_version_satisfies_req("0.6.0", "^0.6.5") returns True.

        This documents the approximation rather than endorsing it.  Epoch
        comparison sees both as epoch (0, 6, 0) and reports satisfied, even
        though cargo would reject `^0.6.5` as unsatisfied by 0.6.0.
        If someone tightens the comparison to a precise semver check, this
        test is the thing to update — change it to assert False then.
        """
        assert dut._version_satisfies_req("0.6.0", "^0.6.5") is True


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

    def _patch_for_pending_only(self) -> dict:
        """Monkeypatch everything to reproduce the six real PENDING violations."""
        return {
            "workspace_packages": lambda: dict(self.MEMBERS),
            "workspace_dependencies": lambda: dict(self.IN_TREE_DEPS),
            "latest_published_version": _mock_latest_published,
            "published_dependencies": _mock_published_deps_pending,
            "range_still_resolvable": lambda crate, req: True,
            "any_published_version_satisfies": lambda crate, req: True,
        }

    def _patch_for_pending_with_unpublished_deps(self) -> dict:
        """Like _patch_for_pending_only but any_published_version_satisfies
        returns False, exposing in-tree requirements that aren't yet on
        crates.io.
        """
        return {
            "workspace_packages": lambda: dict(self.MEMBERS),
            "workspace_dependencies": lambda: dict(self.IN_TREE_DEPS),
            "latest_published_version": _mock_latest_published,
            "published_dependencies": _mock_published_deps_pending,
            "range_still_resolvable": lambda crate, req: True,
            "any_published_version_satisfies": lambda crate, req: False,
        }

    def test_all_pending_exits_zero_with_blocking(self) -> None:
        """Six real pending violations → exit 0 even with --blocking."""
        ns = argparse.Namespace(blocking=True)
        patches = self._patch_for_pending_only()
        with (
            patch.multiple(dut, **patches),
            patch.object(argparse.ArgumentParser, "parse_args", return_value=ns),
        ):
            exit_code = dut.main()
            assert exit_code == 0

    def test_all_pending_exits_zero_without_blocking(self) -> None:
        """Six real pending violations → exit 0 without --blocking."""
        ns = argparse.Namespace(blocking=False)
        patches = self._patch_for_pending_only()
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

        ns = argparse.Namespace(blocking=True)
        with (
            patch.multiple(
                dut,
                workspace_packages=lambda: dict(self.MEMBERS),
                workspace_dependencies=lambda: deps,
                latest_published_version=_mock_latest_published,
                published_dependencies=_mock_published_deps_pending,
                range_still_resolvable=lambda crate, req: True,
                any_published_version_satisfies=lambda crate, req: True,
            ),
            patch.object(argparse.ArgumentParser, "parse_args", return_value=ns),
        ):
            exit_code = dut.main()
            assert exit_code == 1

    def test_one_blocking_exits_zero_without_blocking(self) -> None:
        """One blocking violation → exit 0 without --blocking."""
        deps = dict(self.IN_TREE_DEPS)
        deps[("ll-hls-runtime", "transmux")] = ("^0.20", "normal")

        ns = argparse.Namespace(blocking=False)
        with (
            patch.multiple(
                dut,
                workspace_packages=lambda: dict(self.MEMBERS),
                workspace_dependencies=lambda: deps,
                latest_published_version=_mock_latest_published,
                published_dependencies=_mock_published_deps_pending,
                range_still_resolvable=lambda crate, req: True,
                any_published_version_satisfies=lambda crate, req: True,
            ),
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

    def test_publish_order_not_blocking(self) -> None:
        """transmux requires media-doctor ^0.6, nothing published satisfies it,
        but media-doctor 0.6.0 is being published this wave → NOT a violation.
        """
        ns = argparse.Namespace(blocking=True)
        with (
            patch.multiple(
                dut,
                workspace_packages=lambda: dict(self.MINIMAL_MEMBERS),
                workspace_dependencies=lambda: dict(self.MINIMAL_IN_TREE_DEPS),
                latest_published_version=_mock_latest_published_minimal,
                published_dependencies=_mock_published_deps_minimal,
                range_still_resolvable=lambda crate, req: True,
                any_published_version_satisfies=lambda crate, req: False,
            ),
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
        ns = argparse.Namespace(blocking=True)
        with (
            patch.multiple(
                dut,
                workspace_packages=lambda: dict(members),
                workspace_dependencies=lambda: dict(self.MINIMAL_IN_TREE_DEPS),
                latest_published_version=_mock_latest_published_minimal,
                published_dependencies=_mock_published_deps_minimal,
                range_still_resolvable=lambda crate, req: True,
                any_published_version_satisfies=lambda crate, req: False,
            ),
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
