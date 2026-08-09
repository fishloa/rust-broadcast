# ST 2110-20/-30 fixture provenance (`fixtures/st2110/`)

## Status: BLOCKED — no fixture generated

No ST 2110-20/-30 fixture was generated. This file records why, so the
search doesn't have to be repeated blind.

### Candidate: Intel Media Transport Library (MTL)

- **Repository:** `https://github.com/OpenVisualCloud/Media-Transport-Library`
- **Licence — verified BSD-3-Clause.** `LICENSE` (root), quoted verbatim
  (the core licence text; the file also appends a non-binding disclaimer
  about FFmpeg/GStreamer being unmodified, separately-licensed, optional
  *dependencies* that MTL doesn't redistribute — that disclaimer doesn't
  change MTL's own licence, but is why GitHub's licence detector reports
  `NOASSERTION`/"Other" instead of cleanly matching `BSD-3-Clause`, the
  same false-negative pattern already seen with `coturn` in
  `fixtures/webrtc/PROVENANCE.md`):

  > BSD 3-Clause License
  >
  > Copyright (c) 2022, Intel Corporation
  > All rights reserved.
  >
  > Redistribution and use in source and binary forms, with or without
  > modification, are permitted provided that the following conditions are
  > met: [... three standard BSD-3-Clause conditions, no endorsement ...]

  Clears the licence bar. The description confirms it implements ST
  2110-20/-22/-30/-40 senders and receivers on "COTS hardware (DPDK,
  AF_XDP)".

### Why it's blocked: hugepages are required even in the non-DPDK path

The brief specifically asked to check for a kernel-socket/AF_XDP/non-DPDK
mode before assuming DPDK/vfio-pci NIC binding was required — MTL has
exactly such a mode (`doc/kernel_socket.md`, `MTL_PMD_KERNEL_SOCKET`,
`"kernel:<ifname>"` interface strings), and it **does** avoid vfio-pci NIC
binding: it runs entirely over the normal kernel network stack on a plain
named interface (a container's own `eth0` would work — no dedicated
NIC, no driver rebind).

However, MTL's own docs are explicit that **hugepages are still required
in this mode**, not just DPDK's other memory-mapped-NIC modes:

> "For the kernel socket backend, the library still requires the use of
> huge pages to ensure efficient memory management."
> — `doc/kernel_socket.md` §2, verbatim
>
> "sudo sysctl -w vm.nr_hugepages=2048" — same section, the exact command
> given.

This is because MTL's core (`mtl_init`) is built on DPDK's EAL for memory
management regardless of which packet-I/O backend (`MTL_PMD_KERNEL_SOCKET`
vs a real DPDK PMD vs AF_XDP) is selected — `doc/run.md` §8.4 "Hugepage Not
Available" documents the EAL-level fatal error (`EAL: FATAL: Cannot get
hugepage information`) that results without them, independent of PMD
choice.

`vm.nr_hugepages` is a **host-global, non-namespaced** kernel VM setting
(there is no "hugepage namespace" the way there's a network namespace) —
setting it from inside a container still writes the real host's value, not
a virtualized per-container copy. The task's own safety constraints name
"persistent hugepages" as an explicit STOP-and-report condition for this
specific production host (it runs live Horti/Mustique/CCTV/Archiver
stacks), so this was **not attempted**: no hugepage sysctl was set, no
container was given the elevated privileges (`--privileged`/`--pid=host`
bind-mounting host `/proc`) that would even be required to set it from a
container in the first place.

**This is a genuine, evidenced "blocked", not a guess**: both the
kernel-socket doc and the general run doc independently confirm hugepages
are unconditional for MTL, not an artifact of the DPDK PMD path alone.

### What would unblock this

Either:
1. Explicit authorization to run `sysctl -w vm.nr_hugepages=<N>` directly
   on the `docker.icomb.place` host (outside any container, since the
   setting isn't containerizable) for the duration of a capture, with an
   explicit revert-and-verify step afterward; or
2. A separate, non-production Linux VM/host with hugepages already
   configured (or safe to configure) for this purpose.

Neither was available within this task's scope, so target 2 stops here.

### Other candidates considered and rejected

- **GStreamer `rtpvrawpay`**: GStreamer's own core/base plugins are
  LGPL-2.1 — copyleft, rejected outright per the hard licence rule (same
  disposition as `Upipe/upipe`'s LGPL `upipe-hbrmt` module in
  `fixtures/st2022/PROVENANCE.md`).
- No other BSD/MIT/Apache-2.0 ST 2110-20/-30 **sender** implementation
  (as opposed to a passive dissector/parser) was found on a general web
  search; see the task report for the search terms tried.
- `cisco/herisson` (used successfully for the ST 2022-6 fixture in
  `fixtures/st2022/`) does **not** give us ST 2110-20 either — see that
  file's closing section for the specific dead-code bug that makes its
  2110-20 output path unreachable.
