# Mutation table

Companion to `mutants_dash_pull.py`: each mutant applies a one-line change to the
production source of `multimux/src/source/dash_pull/`, runs the dash_pull tests,
and reverts. The exact edits and commands live in `mutants_dash_pull.py` (`--dry`
checks that every pattern still matches exactly once). It is not part of the build.
