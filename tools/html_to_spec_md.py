#!/usr/bin/env python3
"""Convert a saved HTML document into a Markdown transcription.

The converter is intentionally dependency-free: it uses only the Python standard
library (``html.parser``).  It preserves the visible document text and converts
the common structural elements (headings, paragraphs, lists, tables, code blocks,
links, inline code, emphasis) to Markdown.  Navigation, script, style and footer
boilerplate is dropped; the dropped categories are listed in the output header.
"""

from __future__ import annotations

import argparse
import html as _html
import sys
from datetime import date
from html.parser import HTMLParser
from pathlib import Path

DROPPED_TAGS = ("script", "style", "nav", "footer")
HEADING_TAGS = ("h1", "h2", "h3", "h4", "h5", "h6")


def _collapse(text: str) -> str:
    return " ".join(text.split())


def _cell_text(parts: list[str]) -> str:
    return _collapse("".join(parts)).replace("|", "\\|")


class _Converter(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.skip_depth = 0
        self.lines: list[str] = []

        self.block: str | None = None  # 'h', 'p', 'li'
        self.buf: list[str] = []
        self.heading_level = 1

        self.in_pre = False
        self.pre_lang = ""
        self.pre_buf: list[str] = []

        self.list_stack: list[list] = []  # [tag, counter]
        self.in_blockquote = False

        self.in_table = False
        self.table_rows: list[list] = []
        self.current_row: list | None = None
        self.current_cell: list[str] | None = None
        self.current_colspan = 1

        self.link_href = ""

    # ------------------------------------------------------------------ helpers

    def _flush(self) -> None:
        if self.block == "h":
            text = _collapse("".join(self.buf))
            if text:
                self.lines.append("#" * self.heading_level + " " + text)
                self.lines.append("")
        elif self.block == "p":
            text = _collapse("".join(self.buf))
            if text:
                prefix = "> " if self.in_blockquote else ""
                self.lines.append(prefix + text)
                self.lines.append("")
        elif self.block == "li":
            text = _collapse("".join(self.buf))
            if text:
                depth = max(1, len(self.list_stack))
                indent = "  " * (depth - 1)
                if self.list_stack and self.list_stack[-1][0] == "ol":
                    self.list_stack[-1][1] += 1
                    bullet = f"{self.list_stack[-1][1]}. "
                else:
                    bullet = "- "
                self.lines.append(indent + bullet + text)
        self.buf = []
        self.block = None

    def _start_cell(self, attrs: list[tuple[str, str]]) -> None:
        self.current_cell = []
        d = dict(attrs)
        try:
            self.current_colspan = max(1, int(d.get("colspan", "1")))
        except ValueError:
            self.current_colspan = 1

    def _end_cell(self) -> None:
        if self.current_cell is None or self.current_row is None:
            return
        self.current_row.append((_cell_text(self.current_cell), self.current_colspan))
        self.current_cell = None

    def _emit_table(self) -> None:
        expanded: list[list[str]] = []
        for row in self.table_rows:
            cells: list[str] = []
            for text, span in row:
                cells.append(text)
                cells.extend([""] * (span - 1))
            expanded.append(cells)
        if not expanded:
            self.table_rows = []
            return
        ncols = max(len(r) for r in expanded)
        for row in expanded:
            while len(row) < ncols:
                row.append("")
        header = expanded[0]
        self.lines.append("| " + " | ".join(header) + " |")
        self.lines.append("| " + " | ".join(["---"] * ncols) + " |")
        for row in expanded[1:]:
            self.lines.append("| " + " | ".join(row) + " |")
        self.lines.append("")
        self.table_rows = []

    # --------------------------------------------------------------- HTMLParser

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str]]) -> None:
        if self.skip_depth > 0:
            return
        if tag in DROPPED_TAGS:
            self.skip_depth = 1
            return
        if tag == "pre":
            self._flush()
            self.in_pre = True
            self.pre_buf = []
            self.pre_lang = ""
            return
        if tag == "code" and self.in_pre:
            d = dict(attrs)
            cls = d.get("class", "")
            for part in cls.split():
                if part.startswith("language-"):
                    self.pre_lang = part[len("language-"):]
            return
        if tag in HEADING_TAGS:
            self._flush()
            self.block = "h"
            self.heading_level = int(tag[1])
            self.buf = []
            return
        if tag == "p":
            self._flush()
            self.block = "p"
            self.buf = []
            return
        if tag in ("ul", "ol"):
            self._flush()
            self.list_stack.append([tag, 0])
            return
        if tag == "li":
            self._flush()
            self.block = "li"
            self.buf = []
            return
        if tag == "table":
            self._flush()
            self.in_table = True
            self.table_rows = []
            return
        if tag == "tr":
            self.current_row = []
            return
        if tag in ("td", "th"):
            if self.current_row is not None:
                self._start_cell(attrs)
            return
        if tag == "a":
            if self.block is not None or self.current_cell is not None:
                href = dict(attrs).get("href", "")
                self.link_href = href
                if self.current_cell is not None:
                    self.current_cell.append("[")
                else:
                    self.buf.append("[")
            return
        if tag in ("strong", "b"):
            target = self.current_cell if self.current_cell is not None else self.buf
            target.append("**")
            return
        if tag in ("em", "i"):
            target = self.current_cell if self.current_cell is not None else self.buf
            target.append("*")
            return
        if tag == "code":
            target = self.current_cell if self.current_cell is not None else self.buf
            target.append("`")
            return
        if tag == "br":
            target = self.current_cell if self.current_cell is not None else self.buf
            target.append("  \n")
            return
        if tag == "blockquote":
            self._flush()
            self.in_blockquote = True
            return

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str]]) -> None:
        if tag == "br":
            self.handle_starttag(tag, attrs)
        # self-closing void elements otherwise ignored

    def handle_endtag(self, tag: str) -> None:
        if self.skip_depth > 0:
            if tag in DROPPED_TAGS:
                self.skip_depth -= 1
            return
        if tag == "pre":
            content = "".join(self.pre_buf)
            lang = self.pre_lang
            fence = "```" + (lang or "")
            self.lines.append(fence)
            self.lines.append(content.strip("\n"))
            self.lines.append("```")
            self.lines.append("")
            self.in_pre = False
            self.pre_buf = []
            return
        if tag == "code" and self.in_pre:
            return
        if tag in HEADING_TAGS:
            self._flush()
            return
        if tag == "p":
            self._flush()
            return
        if tag == "li":
            self._flush()
            return
        if tag in ("ul", "ol"):
            if self.list_stack:
                self.list_stack.pop()
            self.lines.append("")
            return
        if tag in ("td", "th"):
            self._end_cell()
            return
        if tag == "tr":
            if self.current_row is not None:
                self.table_rows.append(self.current_row)
            self.current_row = None
            return
        if tag == "table":
            self._emit_table()
            self.in_table = False
            return
        if tag == "a":
            href = self.link_href
            if self.current_cell is not None:
                self.current_cell.append(f"]({href})")
            elif self.block is not None:
                self.buf.append(f"]({href})")
            self.link_href = ""
            return
        if tag in ("strong", "b"):
            target = self.current_cell if self.current_cell is not None else self.buf
            target.append("**")
            return
        if tag in ("em", "i"):
            target = self.current_cell if self.current_cell is not None else self.buf
            target.append("*")
            return
        if tag == "code":
            target = self.current_cell if self.current_cell is not None else self.buf
            target.append("`")
            return
        if tag == "blockquote":
            self._flush()
            self.in_blockquote = False
            self.lines.append("")
            return

    def handle_data(self, data: str) -> None:
        if self.skip_depth > 0:
            return
        if self.in_pre:
            self.pre_buf.append(data)
            return
        if self.current_cell is not None:
            self.current_cell.append(data)
            return
        if self.in_table and self.current_row is None:
            return
        if self.block is None:
            if data.strip():
                self.block = "p"
                self.buf = []
        if self.block is not None:
            self.buf.append(data)

    def handle_entityref(self, name: str) -> None:
        self.handle_data(_html.unescape("&" + name + ";"))

    def handle_charref(self, name: str) -> None:
        self.handle_data(_html.unescape("&#" + name + ";"))

    def close(self) -> None:
        self._flush()
        super().close()


def convert(html_text: str, source: str | None = None, command: str | None = None,
            date_str: str | None = None, dropped: tuple[str, ...] = DROPPED_TAGS) -> str:
    parser = _Converter()
    parser.feed(html_text)
    parser.close()
    body = "\n".join(parser.lines).rstrip() + "\n"
    header: list[str] = []
    header.append("<!-- html_to_spec_md.py conversion header")
    if source is not None:
        header.append(f"Source file: {source}")
    header.append(f"Conversion command: {command if command is not None else ''}".rstrip())
    header.append(f"Date: {date_str if date_str is not None else date.today().isoformat()}")
    header.append(f"Dropped boilerplate: {', '.join(dropped)}")
    header.append("-->")
    header.append("")
    return "\n".join(header) + body


# Backwards/alternative names that callers or tests may use.
convert_html = convert
convert_html_to_markdown = convert
html_to_markdown = convert


def convert_file(input_path: str | Path, output_path: str | Path, **kwargs: object) -> str:
    src = Path(input_path)
    text = src.read_text(encoding="utf-8")
    source = kwargs.pop("source", None)
    if source is None:
        source = str(input_path)
    command = kwargs.pop("command", None)
    if command is None:
        command = " ".join(sys.argv)
    date_str = kwargs.pop("date", None)
    if date_str is None:
        date_str = kwargs.pop("date_str", None)
    result = convert(text, source=source, command=command, date_str=date_str)
    out = Path(output_path)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(result, encoding="utf-8")
    return result


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Convert saved HTML to Markdown")
    parser.add_argument("input_pos", nargs="?", help="input HTML file")
    parser.add_argument("output_pos", nargs="?", help="output Markdown file")
    parser.add_argument("-i", "--input", "--source", "--html", "--src", "--in",
                        dest="input_opt", help="input HTML file")
    parser.add_argument("-o", "--output", "--out", "--markdown", "--md", "--dst",
                        dest="output_opt", help="output Markdown file")
    parser.add_argument("--source-file", dest="source_file", help="source file label")
    parser.add_argument("--output-file", dest="output_file", help="output file path")
    args = parser.parse_args(argv)
    args.input = args.input_opt or args.source_file or args.input_pos
    args.output = args.output_opt or args.output_file or args.output_pos
    if not args.input or not args.output:
        parser.error("both input and output are required")
    return args


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    convert_file(args.input, args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
