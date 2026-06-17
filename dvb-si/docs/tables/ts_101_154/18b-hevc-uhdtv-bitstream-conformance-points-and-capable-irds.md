## Table 18b — HEVC UHDTV Bitstream conformance points and capable IRDs
_§5.14.6, PDF pp. 120-120_

Table 18b: HEVC UHDTV Bitstream conformance points specified in the present document and the IRDs capable to decode them (where "yes" means that the IRD can decode the Bitstream and "no" means that the IRD cannot decode the Bitstream)

| UHDTV Bitstream conformance points | HEVC UHDTV IRD | HEVC HDR UHDTV IRD using HLG10 | HEVC HDR UHDTV IRD using PQ10 | HEVC HDR HFR UHDTV IRD using HLG10 | HEVC HDR HFR UHDTV IRD using PQ10 | HEVC HDR UHDTV2 IRD |
|---|---|---|---|---|---|---|
| SDR — Frame Rate up to 60 Hz — Resolution up to 3840×2160 | yes | yes | yes | yes | yes | yes |
| HDR with PQ10 — Frame Rate up to 60 Hz — Resolution up to 3840×2160 | no | no | yes | no | yes | yes |
| HDR with HLG10 — Frame rate up to 60 Hz — Resolution up to 3840×2160 | yes, but as SDR | yes | yes, but as SDR | yes | yes, but as SDR | yes |
| SDR — HFR with single PID — Resolution up to 3840×2160 | no | no | no | yes | yes | no |
| HDR with PQ10 — HFR with single PID — Resolution up to 3840×2160 | no | no | no | no | yes | no |
| HDR with HLG10 — HFR with single PID — Resolution up to 3840×2160 | no | no | no | yes | yes, but as SDR | no |
| SDR — HFR with dual PID and temporal scalability — Resolution up to 3840×2160 | yes, but at half frame rate | yes, but at half frame rate | yes, but at half frame rate | yes | yes | yes, but at half frame rate |
| HDR with PQ10 — HFR with dual PID and temporal scalability — Resolution up to 3840×2160 | no | no | yes, but at half frame rate | no | yes | yes, but at half frame rate |
| HDR with HLG10 — HFR with dual PID and temporal scalability — Resolution up to 3840×2160 | yes, but as SDR and at half frame rate | yes, but at half frame rate | yes, but as SDR and at half frame rate | yes | yes, but as SDR | yes, but at half frame rate |
| SDR — Frame Rate up to 60 Hz — Resolution up to 7680×4320 | no | no | no | no | no | yes |
| HDR with PQ10 — Frame Rate up to 60 Hz — Resolution up to 7680×4320 | no | no | no | no | no | yes |
| HDR with HLG10 — Frame rate up to 60 Hz — Resolution up to 7680×4320 | no | no | no | no | no | yes |

