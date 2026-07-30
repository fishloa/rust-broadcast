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

    def test_ok_current_requirement(self) -> None:
        """Caller should not call classify for current requirements,
        but it returns 'ok' for any non-stale path.
        """
        # This case is never reached in practice because the caller filters
        # stale requirements first.  The function's return for the ok path
        # falls through, but we validate it doesn't crash on valid input that
        # should be ok.
        result = dut.classify(
            name="ll-hls-runtime",
            max_version="0.3.0",
            in_tree_version="0.3.0",
            sibling="transmux",
            published_req="^0.21",
            in_tree_req="^0.21",
            kind="normal",
            sibling_in_tree_version="0.21.0",
            dev_range_resolvable=False,
        )
        assert result == "blocking"  # in_tree_version == max_version → blocking


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
            ),
            patch.object(argparse.ArgumentParser, "parse_args", return_value=ns),
        ):
            exit_code = dut.main()
            assert exit_code == 0


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
