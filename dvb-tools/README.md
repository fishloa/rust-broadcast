# dvb-tools

[![crates.io](https://img.shields.io/crates/v/dvb-tools.svg)](https://crates.io/crates/dvb-tools)
[![docs.rs](https://img.shields.io/docsrs/dvb-tools)](https://docs.rs/dvb-tools)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

`dvb-tools` is a command-line stream analyzer over the `rust-broadcast` library
crates. It absorbs the former `si_dump` / `t2mi_dump` examples (now `dump` and
`t2mi`) plus a few utilities (`services`, `epg`, `pids`) under one binary.

`dvb-tools` is a std-only binary — not a library crate.

## Subcommands

```text
dvb-tools dump     <file.ts> [--json]                         SI section dump
dvb-tools services <file.ts>                                 SDT + NIT/LCN service tree
dvb-tools epg      <file.ts> [--json]                         EIT schedule
dvb-tools pids     <file.ts>                                 PID table + bitrate
dvb-tools t2mi     <file> [--pid 0xNNN|raw] [--inner] [--plp N]
                                                           T2-MI dump / inner-TS extraction
```

`-h`/`--help`, `<command> --help`, and `-V`/`--version` are auto-generated
(`clap`, per the workspace CLI standard — `docs/CLI-STANDARD.md`).

## Usage

### SI section dump

```console
$ cargo run -p dvb-tools --locked -- dump dvb-si/tests/fixtures/m6-single.ts
pid=0x0000 PROGRAM_ASSOCIATION v0 sn=0
pid=0x0064 PROGRAM_MAP v1 sn=0
-- packets=1264 sections=47 emitted=3 suppressed=44 crc_failures=0 malformed=0

$ cargo run -p dvb-tools --locked -- dump dvb-si/tests/fixtures/m6-single.ts --json
```

### Service tree (SDT + NIT/LCN)

```console
$ cargo run -p dvb-tools --locked -- services dvb-si/tests/fixtures/m6-single.ts
```

### EPG schedule

```console
$ cargo run -p dvb-tools --locked -- epg dvb-si/tests/fixtures/m6-single.ts
$ cargo run -p dvb-tools --locked -- epg dvb-si/tests/fixtures/m6-single.ts --json
```

### PID table + bitrate

```console
$ cargo run -p dvb-tools --locked -- pids dvb-si/tests/fixtures/m6-single.ts
```

### T2-MI dump / inner-TS extraction

```console
$ cargo run -p dvb-tools --locked -- t2mi <file.ts> [--pid 0xNNN|raw] [--inner] [--plp N]

# Extract the inner TS from a T2-MI stream and dump its SI:
$ cargo run -p dvb-tools --locked -- t2mi dvb-si/tests/fixtures/m6-single.ts --inner \
    > inner.ts && \
  cargo run -p dvb-tools --locked -- dump inner.ts
```

## MSRV

**1.86** (workspace minimum). `dvb-tools` is versioned lockstep with the
library crates (`dvb-si`, `dvb-t2mi`, etc.) and ships as part of the 7.x
release series.

## License

MIT OR Apache-2.0, at your option.
