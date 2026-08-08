# dvb-csa test fixtures

## `libdvbcsa-vectors.hex` — ACTIVE ORACLE

Known-answer vectors generated with **libdvbcsa 1.1.0** (VideoLAN's reference
free implementation). Each line is `control_word : plaintext : ciphertext`,
all hex.

Used by `tests/golden_vectors.rs`, which requires byte-identical output from
this crate in both directions. This is the crate's correctness gate — DVB-CSA
has no public normative specification, so agreement with an independent
implementation is the only available standard of proof.

## `france-tnt-scrambled-0x02d0.ts` — NOT USABLE AS AN ORACLE

A 493 KB scrambled TS capture (PID 0x02D0), intended as a second, independent
oracle: scramble an FTA capture with a known control word using TSDuck,
descramble with this crate, require byte-identical recovery.

**The control word it was scrambled with was never recorded.** Without it the
capture cannot be decrypted, so no test can use it, and no test references it.
It is retained only because regenerating a capture is more work than keeping
this one — but as committed it proves nothing.

### To make this a real oracle

1. Take an FTA (unscrambled) TS capture.
2. Scramble it with TSDuck using a control word you choose:
   `tsp -I file plain.ts -P scrambler --cw <16-hex-digits> -O file scrambled.ts`
3. Commit **both** the scrambled output and the control word (record the CW
   here, in this file).
4. Add a test that descrambles with that CW and asserts byte-identical
   recovery of the original.

Until step 3 is done for this file, the crate documents exactly one oracle
(libdvbcsa), which is the honest position.
