# dash-runtime docs

This crate has no spec transcriptions of its own; it relies on the workspace ones:

- `docs/dash-runtime/DESIGN.md` (repository root): the design, decisions D1-D11 and the spec quirks (section 11) that affect the formulas here.
- `docs/specs/dash/23009-1/` (repository root): own-words implementer notes for ISO/IEC 23009-1:2022, with `RECHECK-P0.md` recording the re-read of 5.3.9.5.3, 5.3.9.6 and 5.3.2.1 against the verbatim text.
- `docs/specs/dash/xs-datetime-subset.md`: the accepted `xs:dateTime` subset and its accept/reject table.
- `docs/mutation-table.md` (this directory): the mutants applied to the formulas and the test that kills each.
