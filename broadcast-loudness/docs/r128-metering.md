TECH 3341

LOUDNESS METERING: ‘EBU MODE’
METERING TO SUPPLEMENT EBU
R 128 LOUDNESS NORMALIZATION

SUPPLEMENTARY INFORMATION FOR R128

Geneva
November 2023

Tech 3341 - 2023

‘EBU Mode’ metering to supplement Loudness normalisation

Document History

 EBU Committee

 Drafting Group

 First published

TC

PLOUD

2010

 Revised

  August 2011

 V2; relative gate changed from −8 to −10 LU

  January 2016

 V3; Clarifications, more test signals, updated
 ‘Reference’ section

  November 2023   V4; editorial and formatting changes

Publication Keywords: Audio signal levels, loudness, normalisation, metering.

Acknowledgement

EBU technical publications are the work of experts from EBU Members, Associate Members and third
parties  consisting  of  international  standards  bodies,  industry  partners,  academic  institutions  and
independent consultants.

Their contribution to EBU technical publications is a very generous act by the individuals concerned
and by their employers. The EBU appreciates their efforts and thanks them most sincerely.

This document has been produced with the assistance of the following entities:

EBU  Members:  Florian  Camerer  (ORF),  Andrew  Mason  (BBC  R&D),  Askan  Siegfried  (NDR),  Bosse
Ternstrom (SR), Erik Lundbeck (SVT), Mikko Hahkio (YLE), Matthieu Parmentier (France Television),
Bernard Gabus (RTBF), Andrzej Bialczyk (TVP), Tin Jonker (NPB), Gabriel Leuzinger (SRG SSR), Jean-
Paul  Moerman  (VRT),  Bjørn  Aarseth  (NRK),  Daniele  Airola  Gnota  (RAI),  Frank  Lott  (BR),  Kimio
Hamasaki, (NHK)

Third  parties:  Esben  Skovenborg  (tc  electronic),  Mike  Kahsnitz  (rtw),  Ralph  Kessler  (pinguin
Ingenieurbüro), Thomas Lund (Genelec), Mark Pascoe (Dolby), Paul Tapper (NuGen Audio), Alessandro
Travaglini (FOX Italia), Scott Norcross (CRC)

EBU Project Manager: Frans de Jong

2

Tech 3341 - 2023

‘EBU Mode’ metering to supplement Loudness normalisation

Contents

1.

2.

3.

4.

Introduction ................................................................................... 4

‘EBU Mode’ .................................................................................... 4
2.1  The three time scales .............................................................................................. 4
Integration - times and methods, meter ballistics ............................................................. 5
2.2
2.3  The measurement gate ............................................................................................ 5
2.4  Loudness Range measure .......................................................................................... 6
2.5  Units .................................................................................................................. 6
2.6  True peak measurement ........................................................................................... 6
2.7  Scales and ranges ................................................................................................... 6
2.8  Display requirements............................................................................................... 7
2.9  Calibration, alignment, compliance and accuracy ............................................................. 7
2.10  Various interpretation issues ...................................................................................... 9

References ....................................................................................10

Further reading ..............................................................................10

3

Tech 3341 - 2023

‘EBU Mode’ metering to supplement Loudness normalisation

Loudness Metering: ‘EBU Mode’ metering
to supplement Loudness normalisation
in accordance with EBU R 128

Introduction

1.
The EBU has studied the needs of audio signal levels in production, distribution and transmission of
broadcast  programmes.  It  is  of  the  opinion  that  an  audio-levelling  paradigm  is  needed  based  on
loudness measurement. This is described in EBU Technical Recommendation R 128 [1]. In addition to
the  average  loudness  of  a  programme  (‘Programme  Loudness’)  the  EBU  recommends  that  the
measures ‘Loudness Range’ and ‘Maximum True Peak Level’ be used for the normalisation of audio
signals and to comply with the technical limits of the complete signal chain as well as the aesthetic
needs of each programme/station depending on the genre(s) and the target audience.

In this document the properties of a loudness meter in the so-called ‘EBU Mode’ will be introduced
and  explained  in  detail.  A  set  of  test  signals  providing  minimum  requirements  for  compliance
complements the document.

‘EBU Mode’

2.
A  loudness  meter  may  implement  the  ‘EBU Mode’.  When  ‘EBU Mode’  is  activated  on  a  loudness
meter,  the  meter  shall  comply  with  the  requirements  specified  in  this  document  (as  well  as  the
underlying ITU and EBU recommendations except where differences are explicitly required). Thereby
a  user  could  employ  loudness  meters  from  different  manufacturers  with  a  minimum  of  confusion
caused by differing terminology, scales and measurement methods. A loudness meter may provide
alternatives  to  any  or  all  of  the  ‘EBU Mode’  specifications.  However,  when  such  alternatives  are
selected, the meter will no longer be in ‘EBU Mode’.

The specification of ‘EBU Mode’ does not concern the graphical/UI details or the implementation of
a meter.

The ‘EBU Mode’ is defined by the parameters described in the following sections.

2.1  The three time scales

Regarding time scales, and their terminology:

1.
2.
3.

The shortest time scale is called ‘Momentary’, abbreviated ‘M’.
The intermediate time scale is called ‘Short-term’, abbreviated ‘S’.
The programme- or segment-wise time scale is called ‘Integrated’, abbreviated ‘I’.

In an ‘EBU Mode’ ‘live meter’1, all three time scales shall be available, but not necessarily displayed
at the same time. A ‘non-live’ loudness meter, for example a file-based software meter, which only
implements  a  subset  of  the  ‘EBU Mode’  time  scales,  is  still  considered  compliant,  if  that  subset
complies with the ‘EBU Mode’ requirements.

The loudness meter shall be able to display the maximum value of the ‘Momentary Loudness’ and of
the  ‘Short-term  Loudness’.  These  maximum  values  are  reset  when  the  Integrated  Loudness
measurement is reset.

1 A ‘live meter’ is a meter that can be used in a live environment, measuring an audio signal as it happens. This term is
preferable to ‘real-time meter’ because software analysis of files can be described as ‘real-time’ or as ‘faster than real-
time’, for example.

4

Tech 3341 - 2023

‘EBU Mode’ metering to supplement Loudness normalisation

2.2

Integration - times and methods, meter ballistics

In all cases the measurement is performed as specified of ITU-R BS.1770 [2].

Note:  Where the terms "ITU-R BS.1770" and "ITU-R BS.1771"  are used, the reader should
refer to the most recent versions of the Recommendations published by the ITU-R.

The measurement parameters for ‘EBU Mode’ are:

1.

2.

3.

The Momentary Loudness uses a sliding rectangular time window of length 0.4 s.
The measurement is not gated.
The Short-term Loudness uses a sliding rectangular time window of length 3 s.
The measurement is not gated. The update rate for ‘live meters’ shall be at least 10 Hz.
The Integrated Loudness uses gating as described in ITU-R BS.1770. The update rate for
‘live meters’ shall be at least 1 Hz.

Further  slowdown  of  the  attack  or  release  (decay)  parts  of  the  loudness  signals,  after  the  sliding
rectangular  time  windows,  shall  not  be  employed  in  ‘EBU Mode’  [ITU-R  BS.1771-1  prescribes  a
1st order IIR filter with a time-constant of 0.4 s to measure Momentary Loudness. However, the EBU
has  received  indications  that  a  time-constant  smaller  than  0.4 s  might  be  preferable  with  the  IIR
method (for example, the meter ballistics would better complement the short-term loudness). Hence
the  original  definition  of  Momentary  Loudness  remains  in  EBU  Mode  until  evidence  is  available  to
clarify this issue.].

There may be cases where it is relevant to use other window lengths or time constants than those
specified above. This is allowed in a loudness meter offering ‘EBU Mode’, but it should be clearly
indicated on the meter whether or not the set of EBU parameters are in effect (‘EBU Mode’).

The ‘EBU Mode’ loudness meter shall at least provide functionality that enables the user to –

1.

2.

start/pause/continue  the  measurement  of  Integrated  Loudness  and  Loudness  Range
simultaneously, that is, switch the meter between ‘running’ and ‘stand-by’ states;
reset  the  measurement  of  Integrated  Loudness  and  Loudness  Range  simultaneously,
regardless of whether the meter is in the ‘running’ and ‘stand-by’ state.

2.3  The measurement gate

The ‘Integrated  Loudness’ shall be measured using the gating function specified in ITU-R BS.1770,
summarised as follows:

1.

2.
3.

using  an  absolute  'silence'  gating  threshold  at  −70 LUFS  for  the  computation  of  the
absolute-gated loudness level;
using a relative gating threshold, 10 LU below the absolute-gated loudness level;
The  measurement  input  to  which  the  gating  threshold  is  applied is  the  loudness  of  the
400 ms blocks with a constant overlap between consecutive gating blocks of 75%.

If the end of an integrated loudness measurement lies within a gating block, the incomplete gating
block shall be discarded.

Note:

The gating function excludes from the measurement those blocks of audio that are
below  a  threshold.   For the  relative-threshold based  gating function this  requires
the computation stages described above, as the threshold to be used is itself based
on a measurement of loudness.  In a live meter the integrated loudness has to be
recalculated from the preceding (stored) loudness levels of the blocks from the time
the measurement was started, by recalculating the threshold, then applying it to
the stored values, every time the meter reading is updated.

5

Tech 3341 - 2023

‘EBU Mode’ metering to supplement Loudness normalisation

2.4  Loudness Range measure

The measure ‘Loudness Range’ quantifies the variation in a time-varying loudness measurement; it
measures the variation of loudness on a macroscopic timescale. Loudness Range is supplementary to
the measure of overall loudness, that is, ‘integrated loudness’. The computation of Loudness Range
is based on a measurement of loudness level, as specified in ITU-R BS.1770.

The term ‘Loudness Range’ is abbreviated ‘LRA’. LRA is measured in units of ‘LU’. It is noted that
1 LU is equivalent to 1 dB.

An  ‘EBU Mode’  meter  shall  be  able  to  compute  LRA  for  the  audio  signal  corresponding  to  the
integrated  loudness  measurement.  The  LRA  computation  is  reset  when  the  Integrated  Loudness
measurement is reset.

An ‘EBU Mode’ meter may be able to turn on and off the display of the Loudness Range.

During the first 60 s of the measurement of LRA (for example, after ‘reset’) the meter shall indicate
that the displayed LRA value is not yet to be considered stable. The individual implementation of the
meter may determine the means of providing this indication.

The definition and a reference implementation of the algorithm for calculating ‘Loudness Range’ are
described in EBU Tech 3342 [3].

2.5  Units

The EBU recommends the proposal on naming and units summarized here:

•  A relative measurement, such as relative to a reference level, or a range: LK = xx.x LU
•  An absolute measurement, LK = xx.x LUFS
•  The ‘L’ in ‘LK’ indicates loudness level, the ‘K’ indicates the frequency weighting used.

This notation would resolve the inconsistencies currently present in ITU-R BS.1770 and BS.1771 [4],
and would moreover make them consistent with other existing standards in that area (ISO, IEC).

Note:

The proposal on naming and units is described further in the document ‘Proposal for
the rationalisation of nomenclature used in ITU-R BS.1770 and ITU-R BS.1771’, which
was submitted by the EBU to the ITU-R in April 2010.

2.6  True peak measurement

In addition to the specifications of ITU-R BS.1770 Annex 2, the true-peak measure implemented in an
‘EBU Mode’ meter shall comply with the tolerances specified in Table 1 (signals 15-23), intended as
‘minimum requirements’ tests. The total measurement error of the true-peak level must comply with
these tests, including any pass-band ripple in the upsampling filter and the ‘under-read’ described in
ITU-R BS.1770, Appendix 1 to Annex 2.

2.7  Scales and ranges

The  display  of  an  ‘EBU Mode’  meter  may  simply  be  numerical  or  have  an  indication  on  a  scale.
However, if a scale is shown, it shall meet the following requirements:

An ‘EBU Mode’ meter shall offer two scales, selectable by the user:

1.
2.

range −18.0 LU to +9.0 LU (−41.0 LUFS to −14.0 LUFS), named ‘EBU +9 scale’
range −36.0 LU to +18.0 LU (−59.0 LUFS to −5.0 LUFS), named ‘EBU +18 scale’

This corresponds to the ‘optional’ Requirement PLD-4 of ITU-R BS.1771. The ‘EBU +9 scale’ shall be
used by default.

6

Tech 3341 - 2023

‘EBU Mode’ metering to supplement Loudness normalisation

The scale used may either be an absolute scale, using the unit ‘LUFS’, or alternatively the zero point
may be mapped to some other value, such as the target loudness level (as in ITU-R BS.1771). In the
latter case the unit shall be ‘LU’, indicating a relative scale. For an ‘EBU Mode’ meter, the target
loudness level shall be −23.0 LUFS = 0.0 LU (as defined in EBU R 128). The ‘EBU Mode’ meter shall
offer both the relative and the absolute scale.

The location of the target/reference loudness level shall remain the same, regardless of whether an
absolute or relative scale is displayed.

2.8  Display requirements

The physical properties of the loudness meter, such as size, colours, and design, are not part of the
‘EBU Mode’ specification.

A  minimum  feature  set  is  required  for  all  ‘EBU  Mode’  loudness  meters:  an  ‘EBU  Mode’  compliant
meter  shall  be  able  to  measure  and  display  the  three  main  measures  ‘Programme  Loudness’,
‘Loudness  Range’  and  ‘Maximum  True  Peak  Level’.  For  ‘live  meters’  these  measures  need  not
necessarily be displayed at the same time.

The  ‘EBU Mode’  meter  shall  use  a  display  precision  of  at  most  1  decimal  place  in  all  numerical
loudness readouts (Integrated Loudness or Loudness Range, for example).

The display of the Integrated Loudness shall be in units of LU or LUFS. If absolute and relative scales
are switched, the unit of the display of Integrated Loudness shall be switched accordingly. The unit,
whether LUFS or LU, shall be displayed for all values and scales, at all times.

The ‘EBU Mode’ does not specify what the ‘Integrated Loudness’ meter should indicate until there is
sufficient input data to display a valid result.

The time-scale abbreviations ‘M’ and ‘S’ used in this document are the same as those for ‘mid’ and
‘side’ in other contexts. Alternatives, for example ‘MLK’ and ‘SLK’, have been suggested for use where
ambiguity is thought likely.

2.9  Calibration, alignment, compliance and accuracy

Calibration and alignment:

The stereo 1 kHz, 0 dBFS example signal mentioned in ITU-R BS.1770 would be quite loud to listen
to. However, the definition of the algorithm means that a given attenuation of the input signal results
in the same reduction in the measured result.

For a basic calibration and alignment check of signal level, a 1 kHz stereo sine-wave (signal applied
in  phase  to both  channels  simultaneously), with  its  peak  level  at  −18 dBFS,  is  recommended.  The
meter should read −18.0 LUFS.

The alignment procedure is defined in EBU Tech 3343 ‘Guidelines for Production’ [5].

Note:  A frequency of 1 kHz is used, but as this frequency lies on a filter slope within the
algorithm,  the  calibration  is  more  critical  than  necessary  with  respect  both  to
implementation  accuracy  of  the  filter  and  to  the  accuracy  of  the  calibration
frequency. An error in the frequency of the 1 kHz tone can lead to a result different
from that expected.

Minimum requirements, compliance test:

The typical user of an ‘EBU Mode’ loudness meter will most likely never have the need for performing
a compliance test. Thus, a ‘minimum requirements’ test set is considered sufficient.

If a loudness meter offering ‘EBU Mode’ does not pass these ‘minimum requirements’ tests and the
tests in ITU-R BS.2217-1, there is a considerable risk that the meter is not compliant with ‘EBU Mode’.
If, on the other hand, a meter does pass these tests this does not imply that the meter is sufficiently
accurate in all respects of its implementation.

7

Tech 3341 - 2023

‘EBU Mode’ metering to supplement Loudness normalisation

Note:  In Report ITU-R BS.2217 [6] the ITU provides a set of test signals and tolerances for the
loudness measure of ITU-R BS.1770. The following test signals have been prepared for
the  benefit  of  EBU  members,  to  supplement  the  tests  in  [6]  for  ‘EBU  Mode’  meters.
However, it should be noted that definitions of compliance tests for the measurement
method specified in ITU-R BS.1770 do not, strictly speaking, belong to the scope of this
document, and might subsequently be replaced by a corresponding ITU recommendation.

Table 1: Minimum requirements test signals

Test
case

Test signal

Expected response and
accepted tolerances

1

2

3

4

5

6

7

8

9

10

11

12

13

14

Stereo sine wave, 1000 Hz, −23.0 dBFS (per-channel peak level);
signal applied in phase to both channels simultaneous; 20 s duration

M, S, I = −23.0 ±0.1 LUFS
M, S, I = 0.0 ±0.1 LU

As #1 at −33.0 dBFS

M, S, I = −33.0 ±0.1 LUFS
M, S, I = −10.0 ±0.1 LU

3 tones similar to #1 but with the following durations and levels:
10 s at −36.0 dBFS; 60 s at −23.0 dBFS; 10 s at −36.0 dBFS

I = −23.0 ±0.1 LUFS
I = 0.0 ±0.1 LU

5 tones similar to #1 but with the following durations and levels:
10 s at −72.0 dBFS; 10 s at −36.0 dBFS; 60 s at −23.0 dBFS; 10 s at
−36.0 dBFS; 10 s at −72.0 dBFS

I = −23.0 ±0.1 LUFS
I = 0.0 ±0.1 LU

3 tones similar to #1 but with the following durations and levels:
20 s at −26.0 dBFS; 20.1 s at −20.0 dBFS; 20 s at −26.0 dBFS

I = −23.0 ±0.1 LUFS
I = 0.0 ±0.1 LU

5.0 channel sine wave, 1000 Hz, 20 s duration, with per-channel peak
levels as follows:
  −28.0 dBFS in L and R
  −24.0 dBFS in C
  −30.0 dBFS in Ls and Rs

I = −23.0 ±0.1 LUFS
I = 0.0 ±0.1 LU

Authentic programme 1, stereo, narrow loudness range (NLR)
programme segment; similar in genre to a commercial/promo

Authentic programme 2, stereo, wide loudness range (WLR)
programme segment; similar in genre to a movie/drama

I = −23.0 ±0.1 LUFS
I = 0.0 ±0.1 LU

I = −23.0 ±0.1 LUFS
I = 0.0 ±0.1 LU

2 tones similar to #1 but with the following durations and levels:
(1.34 s at −20.0 dBFS; 1.66 s at −30.0 dBFS) repeated 5 times

S = −23.0 ±0.1 LUFS,
constant after 3 s

For file-based meters; 20 segments with tones similar to #1 but with
the following durations and levels:
(i * 0.15 s of silence; 3 s at −23.0 dBFS; 1 s of silence)
for i = 0, 1, 2, …, 19

For ‘live’ meters; 20 tones similar to #1 but with the following
durations and levels:
(i * 0.15 s of silence; 3 s at −38.0+i dBFS; 3 – i * 0.15 s of silence)
for i = 0, 1, 2, …, 19

Max S = −23.0 ±0.1 LUFS, for
each segment

Max S = −38.0, −37.0, −36.0,
…, −19.0 ±0.1 LUFS,
successive values

2 tones similar to #1 but with the following durations and levels:
(0.18 s at −20.0 dBFS; 0.22 s at −30.0 dBFS) repeated 25 times

M = −23.0 ±0.1 LUFS,
constant after 1 s

For file-based meters; 20 segments with tones similar to #1 but with
the following durations and levels:
(i * 20 ms of silence; 400 ms at −23.0 dBFS; 1 s of silence)
for i = 0, 1, 2, …, 19

For ‘live’ meters; 20 tones similar to #1 but with the following
durations and levels:
(i * 20 ms of silence; 400 ms at −38.0+i dBFS; 400 – i * 20 ms of
silence) for i = 0, 1, 2, …, 19

Max M = −23.0 ±0.1 LUFS,
for each segment

Max M = −38.0, −37.0,
−36.0, …, −19.0 ±0.1 LUFS,
successive values

8

Tech 3341 - 2023

‘EBU Mode’ metering to supplement Loudness normalisation

15

16

17

18

19

20

Stereo sine wave with frequency fs/4 Hz, amplitude 0.50 FFS2, phase
0.0 degrees. The frequency fs/4 denotes 12 kHz for a sample-rate of
48 kHz, etc. The duration of the synthesized tone does not matter,
but the tone should be tapered with a 10 ms fade-in and fade–out.

Max. true-peak level = −6.0
+0.2/−0.4 dBTP

Stereo sine wave with frequency fs/4 Hz, amplitude 0.50 FFS, phase
45.0 degrees

Max. true-peak level = −6.0
+0.2/−0.4 dBTP

Stereo sine wave with frequency fs/6 Hz, amplitude 0.50 FFS, phase
60.0 degrees

Max. true-peak level = −6.0
+0.2/−0.4 dBTP

Stereo sine wave with frequency fs/8 Hz, amplitude 0.50 FFS, phase
67.5 degrees

Max. true-peak level = −6.0
+0.2/−0.4 dBTP

Stereo sine wave with frequency fs/4 Hz, amplitude 1.41 FFS, phase
45.0 degrees

Max. true-peak level = +3.0
+0.2/−0.4 dBTP

Stereo sine wave with frequency fs/6 Hz, amplitude 0.50 FFS,
containing a single period of a sine wave with frequency fs/4,
amplitude 1.00; the signal being continuous in phase at both sides of
the single period.
The signal is synthesized at 4*fs (for example, 192 kHz), and then
lowpass (anti-aliasing) filtered and downsampled to fs with a 0
samples offset. The total duration of the synthesized tone does not
matter, but the tone should be tapered with a short fade-in and
fade–out.

21

As #20, but downsampled with a 1 samples offset (at the 4*fs rate).

22

As #20, but downsampled with a 2 samples offset (at the 4*fs rate).

23

As #20, but downsampled with a 3 samples offset (at the 4*fs rate).

Max. true-peak level = 0.0
+0.2/−0.4 dBTP

Max. true-peak level = 0.0
+0.2/−0.4 dBTP

Max. true-peak level = 0.0
+0.2/−0.4 dBTP

Max. true-peak level = 0.0
+0.2/−0.4 dBTP

The loudness meter shall be reset before each measurement.

Note that test cases 10 and 11 complement each other: As file-based meters can measure a signal
with an exact starting time, test 10 is split into 20 different files to be measured individually. In
contrast, 'live' meters can perform a real-time measurement with multiple meter readings; hence
test 11 consists of one (longer) file that must produce 20 successive values, as specified in Table 1.
The same duality applies to test cases 13 and 14.

Warning: Test signals 15-23 are very loud and should not be listened to at normal playback levels.

Minimum requirements for test signals for the Loudness Range measure are described more fully in
EBU Tech 3342.

These  ‘minimum  requirements  test  signals’  are  available  for  download  from  the  EBU  Technical
website [7], synthesized at a sampling rate of 48 kHz.

2.10  Various interpretation issues

ITU-R BS.1770  does  not  include  the  LFE  channel  in  the  measurement.  The  appropriate  gain  and
frequency-weighting for the LFE channel has been the subject of some discussion and investigation
[8]. It is possible that future revisions of ITU-R BS.1770 will take the LFE channel into account.

The EBU recommends that, if the LFE channel were included in the loudness measurement it should
be weighted by +10 dB to compensate for the fact that the playback gain of the LFE channel is 10 dB
higher in its respective frequency range than the broadband channels (‘in-band gain’).

2 The unit FFS means ”fraction full scale”.

9

Tech 3341 - 2023

‘EBU Mode’ metering to supplement Loudness normalisation

Until the inclusion of the LFE channel is standardised in ITU-R BS.1770, it shall not be included in an
'EBU Mode' loudness meter. If the LFE is included, then this should be clearly indicated on the meter,
since it is not compliant with ITU-R BS.1770, and therefore no longer 'EBU Mode'.

In ITU-R BS.1771, the ‘Momentary Loudness’ term was adopted by the ITU and redefined to use an
IIR  filter  for  measurement.  Hence  measurements  of  Maximum  Momentary  Loudness  using  the
‘EBU Mode’ definition and using the BS.1771 definition can differ by up to 2 LU.

The 1st order IIR filter employed in measuring the ITU-R BS.1771 definition of ‘Momentary Loudness’
shall be implemented such that the lowpass filter and its corresponding time-constant applies to the
squared samples. Annex 2, p.11 of ITU-R BS.1771 may be misleading in this respect (i.e. it is not the
output of eq.2 in BS.1770 that is to be filtered).

An ‘EBU Mode’ meter may optionally display the Momentary Loudness and/or Short-term Loudness of
individual  channels,  in  addition  to  the  channel-summed  loudness  level.  The  specification  of  this
feature is outside the scope of ‘EBU Mode’, but any manufacturer implementing it is encouraged to
declare which channel-weights are used for the loudness levels displayed for the individual channels.

3.

References

[1]  EBU R 128

‘Loudness normalisation and permitted maximum level of audio signals’

[2]

ITU-R BS.1770

‘Algorithms to measure audio programme loudness and true-peak audio
level’

[3]  EBU Tech 3342  ‘Loudness Range: A measure to supplement loudness normalisation in

accordance with EBU R 128’

[4]

ITU-R BS.1771

‘Requirements for loudness and true-peak indicating meters’

[5]  EBU Tech 3343  ‘Guidelines for Production of Programmes in accordance with EBU R 128’

[6]

ITU-R BS.2217

‘Compliance material for Recommendation ITU-R BS.1770’

[7]

tech.ebu.ch

‘Minimum requirements test signals for ‘EBU Mode’ loudness meters’,
available from the EBU at http://tech.ebu.ch/loudness

[8]  aes.org

‘Investigations on the Inclusion of the LFE Channel in the ITU-R BS.1770-1
Loudness Algorithm’, Norcross, Scott G., Lavoie, Michel C.; 127th AES
Convention (October 2009) Paper Number: 7829

4.

Further reading

EBU Tech 3344

‘Guidelines for Distribution and Reproduction of Programmes in accordance
with EBU R 128’

10


