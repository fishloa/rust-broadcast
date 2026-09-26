# ts-fix test fixtures

- `m6-single.ts` — copy of the workspace fixture `fixtures/ts/m6-single.ts`.
- `pat-with-nit.ts` — the workspace fixture `fixtures/ts/h264_aac.ts` with one PAT
  entry added (`program_number` 0 → `network_PID` 0x0010) by TSDuck 3.44-4676:
  `tsp -I file fixtures/ts/h264_aac.ts -P pat --nit 16 -O file ts-fix/tests/fixtures/pat-with-nit.ts`.
  Used as the input for the TSDuck oracle tests in `../tsduck_oracle.rs`. Same licence as
  its source fixture (the workspace licence).
