# AC-4 dac4 oracle (#431)

`dac4` body (29 bytes) — the AC4SpecificBox (ac4_dsi_v1), the byte-exact reference
transmux's built dac4 must match:

    20a401400000001fffffffe0010ff88000004200000250100000030080

Source: the `dac4` box in a real Dolby AC-4 init segment
(`Audio_ID_2ch_64kbps_25fps_ac4.mp4`) from the **Dolby AC-4 Online Delivery Kit
v1.5** — proprietary, **not redistributable** (do NOT vendor the .mp4). Re-derive
locally: fetch the kit (see ../rust-ac4/fixtures/dolby/fetch.sh), then extract the
`dac4` box body. The 29 oracle bytes here are factual box-layout data (like the
esds/dac3 oracles), used as the gate constant.

Real AC-4 **elementary stream** for syncframe/TOC parsing: `ac4_channel.ac4`
(Chromium BSD-3, see NOTICE). Reference TOC parser: ../rust-ac4 `ac4-si` crate.
