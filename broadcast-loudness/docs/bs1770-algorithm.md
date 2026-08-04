Recommendations

International Telecommunication Union

Radiocommunication Sector

Recommendation ITU-R BS.1770-5
(11/2023)

BS Series: Broadcasting service (sound)

Algorithms to measure audio
programme loudness and true-peak
audio level

ii

Rec. ITU-R  BS.1770-5

Foreword

The role of the Radiocommunication Sector is to ensure the rational, equitable, efficient and economical use of the radio-
frequency spectrum by all radiocommunication services, including satellite services, and carry out studies without limit
of frequency range on the basis of which Recommendations are adopted.

The  regulatory  and  policy  functions  of  the  Radiocommunication  Sector  are  performed  by  World  and  Regional
Radiocommunication Conferences and Radiocommunication Assemblies supported by Study Groups.

Policy on Intellectual Property Right (IPR)

ITU-R  policy  on  IPR  is  described  in  the  Common  Patent  Policy  for  ITU-T/ITU-R/ISO/IEC referenced  in  Resolution
ITU-R  1.  Forms  to  be  used  for  the  submission  of  patent  statements  and  licensing  declarations  by  patent  holders  are
available from http://www.itu.int/ITU-R/go/patents/en where the Guidelines for Implementation of the Common Patent
Policy for ITU-T/ITU-R/ISO/IEC and the ITU-R patent information database can also be found.

Series of ITU-R Recommendations

(Also available online at https://www.itu.int/publ/R-REC/en)

Title

Satellite delivery
Recording for production, archival and play-out; film for television
Broadcasting service (sound)
Broadcasting service (television)
Fixed service
Mobile, radiodetermination, amateur and related satellite services
Radiowave propagation
Radio astronomy
Remote sensing systems
Fixed-satellite service
Space applications and meteorology
Frequency sharing and coordination between fixed-satellite and fixed service systems
Spectrum management
Satellite news gathering
Time signals and frequency standards emissions
Vocabulary and related subjects

Series

BO
BR
BS
BT
F
M
P
RA
RS
S
SA
SF
SM
SNG
TF
V

Note: This ITU-R Recommendation was approved in English under the procedure detailed in Resolution ITU-R 1.

All rights reserved. No part of this publication may be reproduced, by any means whatsoever, without written permission of ITU.

© ITU 2023

Electronic Publication
Geneva, 2023

Rec. ITU-R  BS.1770-5

1

RECOMMENDATION  ITU-R  BS.1770-5

Algorithms to measure audio programme loudness and true-peak audio level

(Question ITU-R 135-2/6)

(2006-2007-2011-2012-2015-2023)

Scope

This  Recommendation  specifies  audio  measurement  algorithms  for  the  purpose  of  determining  subjective
programme loudness, and true-peak signal level.

Keywords

3/2 multichannel sound system, advanced sound system, loudness, true-peak signal level

The ITU Radiocommunication Assembly,

considering

a)

that modern digital sound transmission techniques offer an extremely wide dynamic range;

b)
that modern digital sound production and transmission techniques provide a mixture of mono,
stereo  and  3/2  multichannel  formats  specified  in  Recommendation  ITU-R BS.775  and  advanced
sound formats including channel-, object- and scene-based input signals and their combination with
metadata specified in Recommendation ITU-R BS.2051, and that sound programmes are produced in
all of these formats;

that listeners desire the subjective loudness of audio programmes to be uniform for different

c)
sources and programme types;

d)
that  many  methods  are  available  for  the  measurement  of  audio  levels  but  that  existing
measurement methods employed in programme production do not provide the indication of subjective
loudness;

e)
that, for the purpose of loudness control in programme exchange, in order to reduce audience
annoyance, it is essential  to have a single recommended algorithm for the  objective estimation of
subjective loudness;

that  future  complex  algorithms  based  on  psychoacoustic  models  may  provide  improved

f)
objective measures of loudness for a wide variety of audio programmes;

g)

that digital media overloads abruptly, and thus even momentary overload should be avoided,

considering further
that peak signal levels may increase due to commonly applied processes such as filtering or

a)
bit-rate reduction;
b)
signal since the true-peak value may occur between samples;

that existing metering technologies do not reflect the true-peak level contained in a digital

that the state of digital signal processing makes it practical to implement an algorithm that

c)
closely estimates the true-peak level of a signal;

that the use of a true-peak level indicating algorithm will allow accurate indication of the

d)
headroom between the peak level of a digital audio signal and the clipping level,

2

Rec. ITU-R  BS.1770-5

recommends

1
that when an objective measure of the loudness of an audio channel or programme, produced
with  up  to  five  main  channels  per  Recommendation  ITU-R BS.775  (mono  source,  stereo  and  3/2
multichannel  sound),  is  required  to  facilitate  programme  delivery  and  exchange,  the  algorithm
specified in Annex 1 should be used;

2
that  when  an  objective  measure  of  the  loudness  of  an  audio  programme  produced  with  a
larger number of channels (such as the channel configurations specified in Recommendation ITU-R
BS.2051) is required, the algorithm specified in Annex 3 should be used;

3
that when an objective measure of the loudness of object-based audio signals or combination
of channel- and object-based audio signals is required, the algorithm specified in Annex 4 should be
used;

that methods employed in programme production and post-production to indicate programme

4
loudness may be based on the algorithm specified in Annexes 1, 3 and 4;

5
that  when  an  indication  of  true-peak  level  of  a  digital  audio  signal  is  required,  the
measurement method should be based on the guidelines shown in Annex 2, or on a method that gives
similar or superior results,

further recommends

1
that consideration should be given to the possible need to update this Recommendation in the
event that new loudness algorithms are shown to provide performance that is significantly improved
over the algorithm specified in Annexes 1, 3 and 4;

that this Recommendation should be updated when new algorithms have been developed to

2
enable the measurement of audio programme loudness for scene-based audio programmes.

NOTE 1 – Users should be aware that measured loudness is an estimation of subjective loudness and
involves some degree of uncertainty depending on listeners, audio material and listening conditions.

NOTE 2 – For testing compliance of meters according to this Recommendation, test material from
the set described in Report ITU-R BS.2217 may be used.

Annex 1

Specification of the objective multichannel loudness measurement algorithm

This Annex specifies the multichannel loudness measurement modelling algorithm.

The algorithm consists of four stages:
–
–
–

“K” frequency weighting;
mean square calculation for each channel;
channel-weighted summation (surround channels have larger weights, and the LFE channel
is excluded);
gating of 400 ms blocks (overlapping by 75%), where two thresholds are used:
–
–

the first at −70 LKFS;
the second at −10 dB relative to the level measured after application of the first threshold.

–

Rec. ITU-R  BS.1770-5

3

Figure 1 shows a block diagram of the various components of the algorithm. Labels are provided at
different points along the signal flow path to aid in the description of the algorithm. The block diagram
shows inputs for five main channels (left, centre, right, left surround and right surround); this allows
monitoring of programmes containing from one to five channels. For a programme that has less than
five channels some inputs would not be used. The low frequency effects (LFE) channel is not included
in the measurement.

Simplified block diagram of multichannel loudness algorithm

FIGURE 1

The first step of the algorithm applies a 2-stage pre-filtering1 of the signal. The first stage of the pre-
filtering accounts for the acoustic effects of the head, where the head is modelled as a rigid sphere.
The response is shown in Fig. 2.

____________________

1  The K-weighting filter is composed of two stages of filtering; a first stage shelving filter and a second stage

high-pass filter.

4

Rec. ITU-R  BS.1770-5

Response of stage 1 of the pre-filter used to account for the acoustic effects of the head

FIGURE 2

Stage  1  of  the  pre-filter  is  defined  by  the  filter  shown  in  Fig.  3  with  the  coefficients  specified  in
Table 1.

FIGURE 3

Signal flow diagram as a 2nd order filter

TABLE 1

Filter coefficients for stage 1 of the pre-filter to model a spherical head

a1

a2

−1.69065929318241

0.73248077421585

b0

b1

b2

1.53512485958697

−2.69169618940638

1.19839281085285

These filter coefficients are for a sampling rate of 48 kHz. Implementations at other sampling rates
will  require  different  coefficient  values,  which  should  be  chosen  to  provide  the  same  frequency
response that the specified filter provides at 48 kHz. The values of these coefficients may need to be
quantized  due  to  the  internal  precision  of  the  available  hardware.  Tests  have  shown  that  the
performance of the algorithm is not sensitive to small variations in these coefficients.

Rec. ITU-R  BS.1770-5

5

The second stage of the pre-filter applies a simple high-pass filter as shown in Fig. 4.

The stage weighting curve is specified as a second order filter as shown in Fig. 3, with the coefficients
specified in Table 2.

These filter coefficients are for a sampling rate of 48 kHz. Implementations at other sampling rates
will  require  different  coefficient  values,  which  should  be  chosen  to  provide  the  same  frequency
response that the specified filter provides at 48 kHz.

FIGURE 4

Second stage weighting curve

TABLE 2

Filter coefficients for the second stage weighting curve

a1

a2

−1.99004745483398

0.99007225036621

b0

b1

b2

1.0

−2.0

1.0

The power, the mean square of the filtered input signal in a measurement interval T is measured as:

(1)

where yi is the input signal (filtered by the two-stage pre-filter as described above), and i  I where
I = {L,R,C,Ls,Rs}, the set of input channels.

The loudness over the measurement interval T is defined as:

tyTzTiid102=

6

Rec. ITU-R  BS.1770-5

Loudness, LK = –0.691 + 10 log10

                LKFS

(2)

where Gi are the weighting coefficients for the individual channels.

To calculate a gated loudness measurement, the interval T is divided into a set of overlapping gating
block intervals. A gating block is a set of contiguous audio samples of duration Tg = 400 ms, to the
nearest sample. The overlap of each gating block shall be 75% of the gating block duration.

The  measurement  interval  shall  be  constrained  such  that  it  ends  at  the  end  of  a  gating  block.
Incomplete gating blocks at the end of the measurement interval are not used.
The power, the mean square of the jth gating block of the ith input channel in the interval T is:

     where           step = 1-overlap

and

The jth gating block loudness is defined as:

(3)

(4)

For a gating threshold Γ there is a set of gating block indices Jg = {j : lj > Γ} where the gating block
loudness is above the gating threshold. The number of elements in Jg is |Jg|.

The gated loudness of the measurement interval T is then defined as:

(5)

A two-stage process is used to make a gated measurement, first with an absolute threshold, then with
a relative threshold. The gating blocks below the absolute threshold are not used in the relative gating
calculation.  The  relative  threshold  Γr  is  calculated  by  measuring  the  loudness  using  the  absolute
threshold, Γa = –70 LKFS, and subtracting 10 from the result, thus:

where:

(6)

iiizG+=)1(2d1stepjTstepjTigijggtyTzstepTTTjgg–,...2,1,0ijiijzGl+=10log10691.0–LKFSzJGLloudnessGatedgJijgiiKG+=1log10691.0–,10LKFSzJGgJijgiir10–1log10691.0 –10+=LKFSljJaajg70–:==

Rec. ITU-R  BS.1770-5

7

The gated loudness can then be calculated using Γr:

where:

Jg = { j : lj > Γr and lj > Γa }

(7)

The frequency weighting in this measure, which is generated by the pre-filter (concatenation of the
stage  1  filter  to  compensate  for  the  acoustics  effects  of  the  head,  and  the  stage  2  filter,  the  RLB
weighting) is designated K-weighting. The numerical result for the value of loudness that is calculated
in equation (2) should be followed by the designation LKFS. This designation signifies: Loudness,
K-weighted, relative to nominal full scale. The LKFS unit is equivalent to a decibel in that an increase
in the level of a signal by 1 dB will cause the loudness reading to increase by 1 LKFS.

If a 0 dB FS, 1 kHz (997 Hz to be exact, see Notes 1 and 2) sine wave is applied to the left, centre, or
right channel input, the indicated loudness will equal −3.01 LKFS.
NOTE 1 – The constant −0.691 in equation (2) cancels out the K-weighting gain for 997 Hz.

NOTE 2 – IEC 61606 states that unless otherwise specified, the reference frequency for measurement shall be
the actual frequency 997 Hz, which may be stated in non-critical contexts, as the nominal frequency 1 kHz.

The weighting coefficient for each channel is given in Table 3.

TABLE 3

Weightings for the individual audio channels

Channel

Weighting, Gi

Left (GL)

Right (GR)

Centre (GC)

Left surround (GLs)

Right surround (GRs)

1.0 (0 dB)

1.0 (0 dB)

1.0 (0 dB)

1.41 (~ +1.5 dB)

1.41 (~ +1.5 dB)

It  should  be  noted  that  while  this  algorithm  has  been  shown  to  be  effective  for  use  on  audio
programmes that are typical of broadcast content, the algorithm is not, in general, suitable for use to
estimate the subjective loudness of pure tones.

LKFSzJGLloudnessGatedgJijgiiKG+=1log10691.0–,10

8

Rec. ITU-R  BS.1770-5

Attachment 1
to Annex 1
(informative)

Description and development of the multichannel measurement algorithm

This  Attachment  describes  a  newly  developed  algorithm  for  objectively  measuring  the  perceived
loudness of audio signals. The algorithm can be used to accurately measure the loudness of mono,
stereo and multichannel signals. A key benefit of the proposed algorithm is its simplicity, allowing it
to be implemented at very low cost. This Attachment also describes the results of formal subjective
tests  conducted  to  form  a  subjective  database  that  was  used  to  evaluate  the  performance  of  the
algorithm.

1

Introduction

There are many applications where it is necessary to measure and control the perceived loudness of
audio signals. Examples of this include television and radio broadcast applications where the nature
and  content  of  the  audio  material  changes  frequently.  In  these  applications  the  audio  content  can
continually  switch  between  music,  speech  and  sound  effects,  or  some  combination  of  these.
Such changes in the content of the programme material can result in significant changes in subjective
loudness.  Moreover,  various  forms  of  dynamics  processing  are  frequently  applied  to  the  signals,
which can have a significant effect on the perceived loudness of the signal. Of course, the matter of
subjective loudness is also of great importance to the music industry where dynamics processing is
commonly used to maximize the perceived loudness of a recording.

There has been an ongoing effort within Radiocommunication Working Party 6P in recent years to
identify an objective means of measuring the perceived loudness of typical programme material for
broadcast applications. The first phase of ITU-R’s effort examined objective monophonic loudness
algorithms exclusively, and a weighted mean-square measure, Leq(RLB), was shown to provide the
best performance for monophonic signals [A1-2].

It is well appreciated that a loudness meter that can operate on mono, stereo, and multichannel signals
is required for broadcast applications. The present Attachment proposes a new loudness measurement
algorithm that successfully operates on mono, stereo, and multichannel audio signals. The proposed
algorithm is based on a straightforward extension of the  Leq(RLB) algorithm. Moreover, the new
multichannel algorithm retains the very low computational complexity of the monophonic Leq(RLB)
algorithm.

2

Background

In the first phase of the ITU-R study a subjective test method was developed to examine loudness
perception of typical monophonic programme materials [A1-2]. Subjective tests were conducted at
five sites around the world to create a subjective database for evaluating the performance of potential
loudness  measurement  algorithms.  Subjects  matched  the  loudness  of  various  monophonic  audio
sequences to a reference sequence. The audio sequences were taken from actual broadcast material
(television and radio).

In  conjunction  with  these  tests,  a  total  of  ten  commercially  developed  monophonic  loudness
meters/algorithms  were  submitted  by  seven  different  proponents  for  evaluation  at  the  Audio
Perception Lab of the Communications Research Centre, Canada.

Rec. ITU-R  BS.1770-5

9

In addition, Soulodre contributed two additional basic loudness algorithms to serve as a performance
baseline [A1-2]. These two objective measures consisted of a simple frequency weighting function,
followed by a mean-square measurement block. One of the two measures, Leq(RLB), uses a high-
pass frequency weighting curve referred to as the revised low-frequency B-curve (RLB).

The other measure, Leq, is simply an unweighted mean-square measure.

Figure 5 shows the results of the initial ITU-R study for the Leq(RLB) loudness meter. The horizontal
axis indicates the relative subjective loudness derived from the subjective database, while the vertical
axis indicates the loudness predicted by the Leq(RLB) measure. Each point on the graph represents
the result for one of the audio test sequences in the test. The open circles represent speech-based audio
sequences, while the stars are non-speech-based sequences. It can be seen that the data points are
tightly clustered around the diagonal, indicating the very good performance of the Leq(RLB) meter.

Leq(RLB) was found to provide the best performance of all of the meters evaluated (although within
statistical significance some of the psychoacoustic-based meters performed as well). Leq was found
to  perform  almost  as  well  as  RLB.  These  findings  suggest  that  for  typical  monophonic  broadcast
material,  a  simple  energy-based  loudness  measure  is  similarly  robust  compared  to  more  complex
measures that may include detailed perceptual models.

Monophonic Leq (RLB) loudness meter versus subjective results (r = 0.982)

FIGURE 5

3

Design of the Leq(RLB) algorithm

The Leq(RLB) loudness algorithm was specifically designed to be very simple. A block diagram of
the Leq(RLB) algorithm is shown in Fig. 6. It consists of a high-pass filter followed by a means to
average the energy over time. The output of the filter goes to a processing block that sums the energy
and computes the average over time.

The purpose of the filter is to provide some perceptually relevant weighting of the spectral content of
the signal. One advantage of using this basic structure for the loudness measures is that all of the
processing can be done with simple time-domain blocks having very low computational requirements.

10

Rec. ITU-R  BS.1770-5

Block diagram of the simple energy-based loudness measures

FIGURE 6

The Leq(RLB) algorithm shown in Fig. 6 is simply a frequency-weighted version of an Equivalent
Sound Level (Leq) measure. Leq is defined as follows:

(8)

where:

xW :
xRef :
T :

signal at the output of the weighting filter
some reference level
length of the audio sequence.

The  symbol  W  in  Leq(W)  represents  the  frequency  weighting,  which  in  this  case  was  the  revised
low-frequency B-curve (RLB).

4

Subjective tests

In  order  to evaluate  potential  multichannel loudness measures it was necessary  to conduct formal
subjective tests in order to create a subjective database. Potential loudness measurement algorithms
could  then  be  evaluated  in  their  ability  to  predict  the  results  of  the  subjective  tests.  The  database
provided perceived loudness ratings for a broad variety of mono, stereo, and multichannel programme
materials.  The  programme  materials  used  in  the  tests  were  taken  from  actual  television  and  radio
broadcasts from around the world, as well as from CDs and DVDs. The sequences included music,
television  and  movie  dramas,  sporting  events,  news  broadcasts,  sound  effects  and advertisements.
Included in the sequences were speech segments in several languages.

4.1

Subjective test set-up

The  subjective  tests  consisted  of  a  loudness-matching  task.  Subjects  listened  to  a  broad  range  of
typical  programme  material  and  adjusted  the  level  of  each  test  item  until  its  perceived  loudness
matched that of a reference signal (see Fig. 7).

The reference signal was always reproduced at a level of 60 dBA, a level found by Benjamin to be a
typical listening level for television viewing in actual homes [A1-1].

dBd1log10)(02210=TRefWtxxTWLeq

Rec. ITU-R  BS.1770-5

11

FIGURE 7

Subjective test methodology

A software-based multichannel subjective test system, developed and contributed by the Australian
Broadcasting Corporation, allowed the listener to switch instantly back and forth between test items
and adjust the level (loudness) of each item. A screen-shot of the test software is shown in Fig. 8. The
level of the test items could be adjusted in 0.25 dB steps. Selecting the button labelled “1” accessed
the reference signal. The level of the reference signal was held fixed.

FIGURE 8

User interface of subjective test system

Using the computer keyboard, the subject selected a given test item and adjusted its level until its
loudness matched the reference signal. Subjects could instantly switch between any of the test items
by  selecting  the  appropriate  key.  The  sequences  played  continuously  (looped)  during  the  tests.
The software  recorded  the  gain  settings  for  each  test  item  as  set  by  the  subject.  Therefore,  the
subjective tests produced a set of gain values (decibels) required to match the loudness of each test
sequence  with  the  reference  sequence.  This  allowed  the  relative  loudness  of  each  test  item  to  be
determined directly.

Prior to conducting the formal blind tests, each subject underwent a training session in which they
became acquainted with the test software and their task in the experiment. Since many of the test
items  contained  a  mixture  of  speech  and  other  sounds  (i.e.  music,  background  noises,  etc.),
the subjects were specifically instructed to match the loudness of the overall signal, not just the speech
component of the signals.

12

Rec. ITU-R  BS.1770-5

During the formal blind tests the order in which the test items were presented to each subject was
randomized. Thus, no two subjects were presented with the test items in the same order. This was
done to eliminate any possible bias due to order effects.

4.2

The subjective database

The  subjective  database  used  to  evaluate  the  performance  of  the  proposed  algorithm  actually
consisted of three separate datasets. The datasets were created from three independent subjective tests
conducted over the course of a few years.

The first dataset consisted of the results from the original ITU-R study where subjects matched the
perceived loudness of 96 monophonic audio sequences. For this dataset, subjective tests were carried
out at five separate sites around the world providing a total of 97 listeners. A three-member panel
made up of Radiocommunication WP 6P SRG3 members selected the test sequences as well as the
reference  item.  The  reference  signal  in  this  experiment  consisted  of  English  female  speech.  The
sequences were played back through a single loudspeaker placed directly in front of the listener.

Following the original ITU-R monophonic study, some of the algorithm proponents speculated that
the  range and type  of signals  used in the  subjective tests  was not  sufficiently  broad. They further
speculated that it was for this reason that the simple Leq(RLB) energy-based algorithm outperformed
all of the other algorithms.

To address this concern, proponents were asked to submit new audio sequences for a further round
of subjective tests. They were encouraged to contribute monophonic sequences that they felt would
be more challenging to the Leq(RLB) algorithm. Only two of the meter proponents contributed new
sequences.

Using these new sequences, formal subjective tests were conducted at the Audio Perception Lab of
the Communications Research Center, Canada. A total of 20 subjects provided loudness ratings for
96  monophonic  sequences.  The  tests  used  the  same  subjective  methodology  used  to  create  the
first dataset, and the same reference signal was also used. The results of these tests formed the second
dataset of the subjective database.

The third dataset consisted of loudness ratings for 144 audio sequences. The test sequences consisted
of  48  monophonic  items,  48  stereo  items,  and  48  multichannel  items.  Moreover,  one  half  of  the
monophonic items were played back via the centre channel (mono), whereas the other half of the
monophonic items were played back via the left and right loudspeakers (dual mono). This was done
to account for the two different manners in which one might listen to a monophonic signal. For this
test,  the  reference  signal  consisted  of  English  female  speech  with  stereo  ambience  and  low-level
background  music.  A  total  of  20  subjects  participated  in  this  test  which  used  the  loudspeaker
configuration specified in Recommendation ITU-R BS.775 and depicted in Fig. 9.

Rec. ITU-R  BS.1770-5

13

FIGURE 9

Loudspeaker configuration used for the third dataset

The first two datasets were limited to monophonic test sequences and so imaging was not a factor. In
the third dataset, which also included stereo and multichannel sequences, imaging was an important
consideration that needed to be addressed. It was felt that it was likely that the imaging and ambience
within  a  sequence  could  have  a  significant  effect  on  the  perceived  loudness  of  the  sequence.
Therefore, stereo and multichannel sequences were chosen to include a broad range of imaging styles
(e.g. centre pan vs. hard left/right, sources in front vs. sources all around) and varying amounts of
ambience (e.g. dry vs. reverberant).

The  fact  that  subjects  had  to  simultaneously  match  the  loudness  of  mono,  dual  mono,  stereo,
and multichannel signals meant that this test was inherently more difficult than the previous datasets
which were limited to mono signals. This difficulty was furthered by the various imaging styles and
varying amounts of ambience. There was some concern that, as a result of these factors, the subjects
could  be  overwhelmed  by  the  task.  Fortunately,  preliminary  tests  suggested  that  the  task  was
manageable, and indeed the 20 subjects were able to provide consistent results.

5

Design of the multichannel loudness algorithm

As  stated  earlier,  the  Leq(RLB)  algorithm  was  designed  to  operate  on  monophonic  signals,
and an earlier study has shown that it is quite successful for this task. The design of a multichannel
loudness algorithm brings about several additional challenges. A key requirement for a successful
multichannel algorithm is that it must also work well for mono, dual mono, and stereo signals. That is,
these formats must be viewed as special cases of a multichannel signal (albeit very common cases).

In  the  present  study  it  is  assumed  that  the  multichannel  signals  conform  to  the  standard
Recommendation ITU-R BS.775 5.1 channel configuration. No effort is made to account for the LFE
channel.

In the multichannel loudness meter, the loudness of each of the individual audio channels is measured
independently by a monophonic Leq(RLB) algorithm, as shown in Fig. 10. However, a pre-filtering
is applied to each channel prior to the Leq(RLB) measure.

14

Rec. ITU-R  BS.1770-5

Block diagram of proposed multichannel loudness meter

FIGURE 10

The  purpose  of  the  pre-filter  is  to  account  for  the  acoustic  effects  that  the  head  has  on  incoming
signals. Here, the head is modelled as a rigid sphere. The same pre-filter is applied to each channel.
The resulting loudness values are then weighted (Gi) according to the angle of arrival of the signal,
and then summed (in the linear domain) to provide a composite loudness measure. The weightings
are used to allow for the fact sounds arriving from behind a listener may be perceived to be louder
than sounds arriving from in front of the listener. The combination of the “pre-filter” and “RLB filter”
in Fig. 10 is referred to as K-weighting as indicated in the main part of Annex 1 above.

A key  benefit of  the  proposed multichannel loudness algorithm is  its simplicity. The algorithm is
made up entirely of very basic signal processing blocks that can easily be implemented in the time-
domain on inexpensive hardware. Another key benefit of the algorithm is its scalability. Since the
processing applied to each channel is identical, it is very straightforward to implement a meter that
can  accommodate  any  number  of  channels  from  1 to N.  Moreover,  since  the  contributions  of  the
individual channels are summed as loudness values, rather than at the signal level, the algorithm does
not depend on inter-channel phase or correlation. This makes the proposed loudness measure far more
generic and robust.

6

Evaluation of the multichannel algorithm

The 336 audio sequences used in the three datasets were processed through the proposed multichannel
algorithm and the predicted loudness ratings were recorded. As a result of this process, the overall
performance  of  the  algorithm  could  be  evaluated  based  on  the  agreement  between  the  predicted
ratings and the actual subjective ratings obtained in the formal subjective tests.

Figures 11, 12 and 13 plot the performance of the proposed loudness meter for the three datasets.
In each  Figure  the  horizontal  axis  provides  the  subjective  loudness  of  each  audio  sequence  in  the
dataset. The vertical axis indicates the objective loudness predicted by the proposed loudness meter.
Each point on the graph represents the result for an individual audio sequence. It should be noted that
a perfect objective algorithm would result in all data points falling on the diagonal line having a slope
of 1 and passing through the origin (as shown in the Figures).

Rec. ITU-R  BS.1770-5

15

FIGURE 11

Results for the first (monophonic) dataset (r = 0.979)

It can be seen from Fig. 11 that the proposed multichannel loudness algorithm performs very well at
predicting  the  results  from  the  first  (monophonic)  dataset.  The  correlation  between  the  subjective
loudness ratings and the objective loudness measure is r = 0.979.

As seen in Fig. 12, the correlation between the subjective loudness ratings and the objective loudness
measure for the second dataset is also very good (r = 0.985). It is interesting to note that about one half
of the sequences in this dataset were music.

16

Rec. ITU-R  BS.1770-5

FIGURE 12

Results for the second (monophonic) dataset (r = 0.985)

Results for the third (mono, stereo and multichannel) dataset (r = 0.980)

FIGURE 13

Figure 13  shows  the  results  for  the  third  dataset,  which  included  mono,  dual  mono,  stereo  and
multichannel  signals.  The  multi-channel  results  included  in  Figs  13  and  14  are  for  the  specified
algorithm, but with the surround channel weightings set to 4 dB (original proposal) instead of 1.5 dB
(final specification). It has been verified that the change from 4.0 dB to 1.5 dB does not have any

Rec. ITU-R  BS.1770-5

17

significant effect on the results. Once again, the performance of the algorithm is very good, with a
correlation of r = 0.980.

It is useful to examine the performance of the algorithm for all of the 336 audio sequences that made
up the subjective database. Therefore, Fig. 14 combines the results from the three datasets. It can be
seen  that  the  performance  is  very  good  across  the  entire  subjective  database,  with an overall
correlation of r = 0.977.

FIGURE 14

Combined results for all three datasets (r = 0.977)

The results of this evaluation indicate that the multichannel loudness measurement algorithm, based
on  the  Leq(RLB)  loudness  measure,  performs  very  well  over  the  336  sequences  of  the  subjective
database. The subjective database provided a broad range of programme material including music,
television and movie dramas, sporting events, news broadcasts, sound effects, and advertisements.
Also included in the sequences were speech segments in several languages. Moreover, the results
demonstrate that the proposed loudness meter works well on mono, dual mono, stereo, as well as
multichannel signals.

References

[A1-1]  BENJAMIN,  E.  (October,  2004)  Preferred  Listening  Levels  and  Acceptance  Windows  for  Dialog
Reproduction in the Domestic Environment, 117th Convention of the Audio Engineering Society, San
Francisco, Preprint 6233.

[A1-2]  SOULODRE, G.A. (May, 2004) Evaluation of Objective Loudness Meters, 116th Convention of the

Audio Engineering Society, Berlin, Preprint 6161.

18

Rec. ITU-R  BS.1770-5

Annex 2

Guidelines for accurate measurement of “true-peak” level

This Annex describes an algorithm for estimation of true-peak level within a single channel linear
PCM digital audio signal. The discussion that follows presumes a 48 kHz sample rate. True-peak
level  is  the  maximum  (positive  or  negative)  value  of  the  signal  waveform  in  the  continuous  time
domain; this value may be higher than the largest sample value in the 48 kHz time-sampled domain.

1

Summary

The stages of processing are:
1

Attenuate: 12.04 dB attenuation

2
3
4
5

2

4  over-sampling
Low-pass filter
Absolute: Absolute value
Conversion to dB TP

Block diagram

3

Detailed description

The first step consists of imposing an attenuation of 12.04 dB (2-bit shift). The purpose of this step
is to provide headroom for the subsequent signal processing that could employ integer arithmetic.
This step is not necessary if the calculations are performed in floating point.

The  4   over-sampling  filter  increases  the  sampling  rate  of  the  signal  from  48 kHz  to  192 kHz.
This higher sample rate version of the signal more accurately indicates the actual waveform that is
represented  by  the  audio  samples.  Higher  sampling  rates  and  over-sampling  ratios  are  preferred
(see Attachment  1  to  this  Annex).  Incoming  signals  that  are  at  higher  sampling  rates  require
proportionately  less  over-sampling  (e.g.  for  an  incoming  signal  at  96  kHz  sample  rate  a
2  over-sampling would be sufficient.)

One  set  of  filter  coefficients  (for  the  order  48,  4-phase,  FIR  interpolating)  that  would  satisfy  the
requirements would be as follows:

Phase 0

Phase 1

Phase 2

Phase 3

0.0017089843750

−0.0291748046875

−0.0189208984375

−0.0083007812500

0.0109863281250

0.0292968750000

0.0330810546875

0.0148925781250

−0.0196533203125

−0.0517578125000

−0.0582275390625

−0.0266113281250

0.0332031250000

0.0891113281250

0.1015625000000

0.0476074218750

−0.0594482421875

−0.1665039062500

−0.2003173828125

−0.1022949218750

0.1373291015625

0.4650878906250

0.7797851562500

0.9721679687500

Rec. ITU-R  BS.1770-5

19

0.9721679687500

0.7797851562500

0.4650878906250

0.1373291015625

−0.1022949218750

−0.2003173828125

−0.1665039062500

−0.0594482421875

0.0476074218750

0.1015625000000

0.0891113281250

0.0332031250000

−0.0266113281250

−0.0582275390625

−0.0517578125000

−0.0196533203125

0.0148925781250

0.0330810546875

0.0292968750000

0.0109863281250

−0.0083007812500

−0.0189208984375

−0.0291748046875

0.0017089843750

The absolute value of the samples is taken by inverting the negative value samples; at this point the
signal is unipolar, with negative values replaced by positive values of the same magnitude.

The result after four stages (attenuation, oversampling, filtering, and taking the absolute value) is a
number in the same domain as the original sample values (for example, 24-bit integer). After this,
it is necessary to compensate for the initial 12.04 dB attenuation. This normalises the overall gain of
the processing to unity.

It must be understood that amplification of the attenuated value by 12.04 dB (2-bit left shift) will, in
general, require conversion of the value into a numeric format capable of representing values higher
than  the  full  scale  range  of  the  original  format.  Performing  the  calculation  steps  in  floating  point
format satisfies this requirement. An alternative to amplification of the result, is to calibrate the meter
scale appropriately.

Meters that follow these guidelines, and that use an oversampled sampling rate of at least 192 kHz,
should  indicate  the  result  in  the  units  of  dB  TP,  having  converted  the  result  to  a  logarithmic
scale.  This can be achieved by calculating “20log10” of the attenuated, oversampled, filtered, absolute
value, then adding 12.04 dB. The “dB TP”. Designation signifies decibels relative to 100% full scale,
true-peak measurement.

Attachment 12
to Annex 2
(informative)

Considerations for accurate peak metering of digital audio signals

What is the problem?

Peak meters in digital audio systems often register “peak-sample” rather than “true-peak”.

A peak-sample meter usually works by comparing the absolute (rectified) value of each incoming
sample with the meter’s current reading; if the new sample is larger it replaces the current reading; if
not, the current reading is multiplied by a constant slightly less than unity to produce a logarithmic
decay.  Such  meters  are ubiquitous  because  they  are  simple  to  implement,  but  they  do  not  always
register the true-peak value of the audio signal.

____________________

2  NOTE 1 – The following informative text was contributed by AES Standards Working Group SC-02-01

through the Radiocommunication WP 6J Rapporteur on loudness metering.

20

Rec. ITU-R  BS.1770-5

So using a peak-sample meter where accurate metering of programme peaks is important can lead to
problems.  Unfortunately,  most  digital  peak  meters  are  peak-sample  meters,  although  this  is  not
usually obvious to the operator.

The problem occurs because the actual peak values of a sampled signal usually occur between the
samples rather than precisely at a sampling instant, and as such are not correctly registered by the
peak-sample meter.

This results in several familiar peak-sample meter anomalies:
–

Inconsistent peak readings: It is often noticed that repeatedly playing an analogue recording
into  a  digital  system  with  a  peak-sample  meter  produces  quite  different  readings  of
programme peaks on each play. Similarly, if a digital recording is repeatedly played through
a sample-rate converter before metering, registered peaks are likewise different on each play.
This is because the sample instants can fall upon different parts of the true signal on each
play.
Unexpected overloads: Since sampled signals may contain overloads even when they have
no samples at, or even close to, digital full scale, overload indication by a peak-sample meter
is unreliable. Overloads may cause clipping in subsequent processes, such as within particular
D/A  converters  or  during  sample-rate  conversion,  even  though  they  were  not  previously
registered by the peak-sample meter (and were even inaudible when monitored at that point).
Under-reading  and  beating  of  metered  tones:  Pure  tones  (such  as  line-up  tones)  close  to
integer  factors  of  the  sampling  frequency  may  under-read  or  may  produce  a  constantly
varying reading even if the amplitude of the tone is constant.

–

–

How bad can the problem be?

In general, the higher the frequency of the peak-sample metered signal, the worse the potential error.

For  continuous  pure  tones  it  is  easy  to  demonstrate,  for  example,  a  3 dB  under-read  for
an unfortunately-phased tone at a quarter of the sampling frequency. The under-read for a tone at half
the sampling frequency could be almost infinite; however most digital audio signals do not contain
significant energy at this frequency (because it is largely excluded by anti-aliasing filters at the point
of  D/A  conversion  and  because  “real”  sounds  are  not  usually  dominated  by  continuous  high
frequencies).

Continuous tones which are not close to low-integer factors of the sampling frequency do not under-
read on peak-sample meters because the beat frequency (the difference between n.ftone and fs) is high
compared to the reciprocal of the decay rate of the meter. In other words, the sampling instant is close
enough to the true-peak of the tone often enough that the meter does not under-read.

However, for individual transients, under-reads are not concealed by that mechanism, so the higher
the frequency content of the transient, the larger the potential under-read. It is normal in “real” sound
for  transients  to  occur  with  significant  high  frequency  content,  and  under-reading  of  these  can
commonly be several dBs.

Because  real  sounds  generally  have  a  spectrum  which  falls  off  towards  higher  frequencies,  and
because this does not change with increasing sampling frequency, peak-sample meter under-read is
less severe at higher original sampling frequencies.

What is the solution?

In  order  to  meter  the  true-peak  value  of  a  sampled  signal  it  is  necessary  to  “over-sample”
(or “up-sample”) the signal, essentially recreating the original signal between the existing samples,
and thus increasing the sampling frequency of the signal. This proposal sounds dubious: how can be
recreated information which appears already to have been lost? In fact, sampling theory shows that it

Rec. ITU-R  BS.1770-5

21

can be done, because it is known that the sampled signal contains no frequencies above half of the
original sampling frequency.

What over-sampling ratio is necessary? A couple of questions need to be answered to find out:
–
–

What is the maximum acceptable under-read error?
What  is  the  ratio  of  the  highest  frequency  to  be  metered  to  the  sampling  frequency
(the maximum “normalized frequency”)?

If these criteria are known, it is possible to calculate the over-sampling ratio needed (even without
considering yet the detail of the over-sampling implementation) by a straightforward “graph-paper”
method. It can be simply considered what under-read will result from a pair of samples at the over-
sampled  rate  occurring  symmetrically  either  side  of  the  peak  of  a  sinusoid  at  our  maximum
normalized frequency. This is the “worst case” under-read.

So for:  over-sampling ratio, n

maximum normalized frequency, fnorm
sampling frequency, fs

it can be seen that:

the sampling period at the over-sampled rate is 1/n.fs
the period of the maximum normalized frequency is 1/fnorm.fs

so:

or:

the maximum under-read (dB) is 20.log(cos(2.π.fnorm.fs/n.fs.2))
(2  in  denominator  since  a  peak  can  be  missed  by  a  maximum  of  half  the  over-sampling
period)

maximum under-read (in dB) = 20.log(cos(π.fnorm/n))

This equation was used to construct the following Table, which probably covers the range of interest:

Over-sampling ratio

Under-read (dB) maximum
fnorm = 0.45

Under-read (dB) maximum
fnorm = 0.5

4

8

10

12

14

16

32

0.554

0.136

0.087

0.060

0.044

0.034

0.008

0.688

0.169

0.108

0.075

0.055

0.042

0.010

How should a true-peak meter be implemented?

The  over-sampling  operation  is  performed  by  inserting  zero-value  samples  between  the  original
samples in order to generate a data stream at the desired over-sampled rate, and then applying a low-
pass “interpolation” filter to exclude frequencies above the desired maximum fnorm. If the peak-sample
algorithm is now operated on the over-sampled signal, a true-peak meter with the desired maximum
under-read is obtained.

22

Rec. ITU-R  BS.1770-5

It is interesting to consider the implementation of such an over-sampler. It is usual to implement such
the low-pass filter as a symmetrical FIR. Where such filters are used to pass high-quality audio, e.g. in
(old-fashioned) over-sampling D/A converters or in sample-rate converters, it is necessary to calculate
a large number of “taps” in order to maintain very low passband ripple, and to achieve extreme stop-
band  attenuation  and  a  narrow  transition  band.  A  long  word-length  must  also  be  maintained  to
preserve dynamic range and minimize distortion.

However, since the output of our over-sampler is not going to be listened to, but only used to display
a reading or drive a bar graph, the same precision requirements are probably not obtained. So as long
as the passband ripple, coupled with addition of spurious components from the stop-band, does not
degrade the reading accuracy beyond our target, the results are satisfying. This reduces the required
number  of  taps  considerably,  although  it  may  still  be  needed  to  achieve  a  narrow  transition  band
depending on our maximum normalized frequency target. Similarly, the word-length may only need
to be sufficient to guarantee our target accuracy down to the bottom of the bar graph, unless accurate
numerical output is required to low amplitudes.

So it may be that an appropriate over-sampler (possibly for many channels) could be comfortably
implemented in an ordinary low-cost DSP or FPGA, or perhaps in an even more modest processor.
On the other hand, over-sampling meters have been implemented using high-precision over-sampling
chips intended for D/A converter use. Whilst this is rather wasteful of silicon and power, the devices
are low-cost and readily available.

The simplest way to determine the required number of taps and the tap coefficients for a particular
meter specification is to use a recursive FIR filter design programme such as Remez or Meteor.

It may also be a requirement in a peak-meter to exclude the effect of any input DC, since audio meters
have traditionally been DC blocked. On the other hand, if the interest is in the true-peak signal value
for  the  purposes  of  overload  elimination,  then  DC  content  must  be  maintained  and  metered.  If
required, exclusion of DC can be achieved with low computation power by inclusion of a low-order
IIR high-pass filter at the meter’s input.

It  is  sometimes  required  to  meter  peak  signal  amplitude  after  the  application  of  some  type  of
weighting  filter  in  order  to  emphasize  the  effects  of  certain  parts  of  the  frequency  band.
Implementation is dependent on the nature of the particular weighting filter.

Annex 3

Extended loudness measurement algorithm for loudspeaker
configurations of advanced sound systems

1

Extension for loudspeaker configurations of advanced sound system

This  section  specifies  the  objective  loudness  measurement  algorithm  for  arbitrarily  placed
loudspeaker configurations of the advanced sound system.

The algorithm is an extension of the basic algorithm for 3/2 multichannel sound system specified in
Annex 1, in which the number of input channels is increased and the third stage of the basic algorithm
is modified as follows:
–

channel-weighted  summation  (each  channel  except  the  LFE  channels  has  a  weighting
coefficient Gi depending on the azimuth and elevation angles of its position).

Rec. ITU-R  BS.1770-5

23

Figure 15 shows a block diagram of the objective loudness measurement algorithm for loudspeaker
configurations of the advanced sound system specified in Recommendation ITU-R BS.2051. N is the
number of input channels excluding the LFE channels. First, second and fourth stages of the algorithm
(filtering and gating procedure) are the same as in the algorithm for the 3/2 multichannel format that
is independent of the channel position.

FIGURE 15

Simplified block diagram of objective loudness measurement algorithm
for loudspeaker configurations of the advanced sound system

The weighting coefficient Gi for a channelʼs position is given in Table 4. Gi depends on the direction
of the channelʼs position, specified by the azimuth angle (θ) and the elevation angle (φ).

TABLE 4

Position-dependent weightings of the channels

Elevation (φ)

|φ| < 30°

else

|θ| < 60°

1.00 (±0 dB)

Azimuth (θ)

60° ≦ |θ| ≦ 120°

120° < |θ| ≦ 180°

1.41 (+1.5 dB)

1.00 (±0 dB)

1.00 (±0 dB)

In accordance with Table 4, the position-dependent weightings of the channels for the loudspeaker
configurations specified in Recommendation ITU-R BS.2051 are defined in Table 5.

24

Rec. ITU-R  BS.1770-5

TABLE 5

Position-dependent weightings for the loudspeaker configurations
specified in Recommendation ITU-R BS.2051

Loudspeaker configuration

Loudspeaker
label

Weighting

A

B

C

D

E

F

G

H

I

J

0+2+0

0+5+0

2+5+0

4+5+0

4+5+1

3+7+0

4+9+0

9+10+3

0+7+0

4+7+0

X

X

X

X

X

X

X

M+000

M+SC

M-SC

M+030

M-030

M+060

M-060

M+090

M-090

M+110

M-110

M+135

M-135

M+180

U+000

U+030

U-030

U+045

U-045

1.00

1.00

1.00

1.00

1.00

1.41

1.41

1.41

1.41

1.41

1.41

1.00

1.00

1.00

1.00

1.00

1.00

1.00

1.00

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(+1.5 dB)

(+1.5 dB)

(+1.5 dB)

(+1.5 dB)

(+1.5 dB)

(+1.5 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

Rec. ITU-R  BS.1770-5

TABLE 5 (end)

Loudspeaker configuration

25

Loudspeaker
label

Weighting

A

B

C

D

E

F

G

H

I

J

0+2+0

0+5+0

2+5+0

4+5+0

4+5+1

3+7+0

4+9+0

9+10+3

0+7+0

4+7+0

U+090

U-090

U+110

U-110

U+135

U-135

U+180

T+000

B+000

B+045

B-045

1.00

1.00

1.00

1.00

1.00

1.00

1.00

1.00

1.00

1.00

1.00

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

(±0.0 dB)

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

X

26

Rec. ITU-R  BS.1770-5

Annex 4

Loudness measurement algorithm for object-based audio signals or combination
of channel- and object-based audio signals

This Annex specifies the objective loudness measurement algorithm for object-based audio signals
or combination of channel- and object-based audio signals including those of the advanced sound
system.

The object-based audio signals or combination of channel- and object-based audio signals should first
be rendered to a specific loudspeaker configuration. For the advanced sound system, a loudspeaker
configuration specified in Recommendation ITU-R BS.2051 should be used. The objective loudness
is measured from the audio signals output from the rendering process using the loudness measurement
algorithms  specified  in  Annexes  1  and  3  of  this  Recommendation.  Figure  16  shows  a  simplified
diagram of the overall loudness measurement process.

The objective loudness may  differ depending on rendering conditions. Therefore, the loudspeaker
configuration and rendering algorithm used in the measurement should be reported.

Simplified block diagram of objective loudness measurement algorithm
for object-based audio signals or combination of channel- and object-based audio signals

FIGURE 16

Rec. ITU-R  BS.1770-5

27

Attachment 1
to Annex 4
(informative)

Objective loudness for object-based audio signals or combination of channel-
and object-based audio signals depending on rendering conditions

This Attachment describes differences in objective loudness depending on rendering conditions for
combination of channel- and object-based audio signals and the results of subjective tests [3].

1

Difference in objective loudness depending on rendering conditions

In a total of eight programmes consisting of multiple audio objects including multilingual dialogues,
boosted dialogues, dialogues for multiple viewing and background sound were prepared as original
sound  materials  of  complete  audio  programmes.  The  original  sound  materials  were  created  using
position  metadata  of  a  polar  coordinate  system  and  a  monitoring  sound  system  that  is  System  H
(9+10+3) as specified in Recommendation ITU-R BS.2051. The original complete sound materials
were  adjusted  so  that  the  objective  loudness  was  −24.0  LKFS  when  rendered  to  the  loudspeaker
configuration  of  System  H  using  the  ITU-R  ADM  Renderer  (IAR)  with  the  polar  coordinate  as
specified in Recommendation ITU-R BS.2127.

31 test materials were created by selecting 12-16 second clips from the original sound materials. The
Cartesian  coordinate  version  of  the  clips  was  created  using  the  conversion  algorithm  specified  in
Recommendation ITU-R BS.2127. A test material was rendered to six loudspeaker configurations
(sound systems A (0+2+0), B (0+5+0), I (0+7+0), D (4+5+0), J (4+7+0) and H (9+10+3)) using five
rendering  algorithms  (IAR  with  the  polar  coordinate  system  (IAR-Polar),  IAR  with  the  Cartesian
coordinate system (IAR-Cartesian), Renderer A and Renderer B with default trim and without trim.
NOTE: Renderer B does not support sound system H (9+10+3). Renderer B with default trim and without trim
behave the same way as that for sound system J (4+7+0).

Table 6 shows differences in objective loudness depending on rendering conditions. Differences in
objective loudness depending on loudspeaker configurations were up to 2.1 to 4.1 LU when a single
rendering  algorithm was used.  On the  other hand, differences in  objective loudness depending on
rendering  algorithms  were  up  to  0.3  (for  9+10+3)  to  5.9  (for  0+2+0,  stereo)  LU  when  a  single
loudspeaker configuration was used. When the number of loudspeakers is smaller than the number of
audio signals, objective loudness is affected by the rendering algorithm. Therefore, objective loudness
with the rendering conditions used in the measurement should be reported.

Difference in objective loudness depending on rendering conditions

TABLE 6

Rendering
algorithm

Max (abs.) / Mean
(abs.)

Loudspeaker
configuration

IAR-Polar

IAR-Cartesian

Renderer A

Renderer B with trim

3.1 / 1.1 LU

3.1 / 1.1 LU

2.1 / 0.6 LU

4.1 / 1.9 LU

0+2+0

0+5+0

0+7+0

4+5+0

Max (abs.) / Mean (abs.)
excluding Renderer B with
trim, or without trim

5.9 / 1.6 LU, 3.6 / 1.9 LU

3.9 / 1.0 LU, 2.9 / 1.7 LU

2.3 / 0.7 LU, 1.7 / 0.9 LU

1.9 / 0.6 LU, 1.7 / 0.9 LU

28

Rec. ITU-R  BS.1770-5

Renderer B without
trim

3.8 / 1.0 LU

4+7+0

1.7 / 0.5 LU, 1.7 / 0.5 LU

9+10+3 (1)

0.3 / 0.0 LU, 0.3 / 0.0 LU

(1)  The result of 9+10+3 does not include data of Renderer B with and without trim.

2

Subjective tests

To evaluate the loudness measurement  algorithm  for channel- and  object-based audio  signals that
require  a  rendering  process  for  playback,  subjective  tests  were  conducted.  The  equipment  and
procedure of subjective tests are based on previous subjective tests (see Annex 1).

2.1

Effects on test materials

To investigate effects on test materials, 31 test materials rendered by IAR-Polar to three loudspeaker
configurations  (0+2+0,  0+5+0  and  9+10+3)  were  used  in  test  1.  Figure  17  shows  plots  of  the
performance of the proposed loudness algorithm for two sets of 31 test materials whose initial values
are different. Compared with results for channel-based audio signals [1], there is no significant effect
on the results even if a set of audio signals after rendering process are used in loudness measurement.

Results for 31 test materials with three loudspeaker configurations

FIGURE 17

2.2

Effects on loudspeaker configurations

To investigate effects on loudspeaker configurations, 15 test materials rendered by IAR-Polar to six
loudspeaker configurations (0+2+0, 0+5+0, 4+5+0, 0+7+0, 4+7+0 and 9+10+3) were used in test 2.
Figure  18  plots  the  performance  of  the  proposed  loudness  algorithm  for  six  loudspeaker
configurations. From the comparison of results among different loudspeaker configurations, there is
no  significant  effect  on  the  results.  The  other  experiment  using  four  loudspeaker  configurations
(0+5+0, 4+5+0, 0+7+0, 4+7+0) shows the same tendency [2].

Rec. ITU-R  BS.1770-5

29

FIGURE 18

Results for six loudspeaker configurations

2.3

Effects on rendering algorithms

To  investigate  effects  on  rendering  algorithms,  five  test  materials  rendered  by  five  rendering
algorithms (IAR-Polar, IAR-Cartesian, Renderer A and Renderer B with and without trim) to four
loudspeaker configurations (0+2+0, 0+5+0, 4+5+0 and 4+7+0) were used in test 3. Figure 19 shows
plots  of  the  performance  of  the  proposed  loudness  algorithm  for  five  rendering  algorithms.  From
comparison  of  results  among  different  rendering  algorithms,  there  is  no  significant  effect  on  the
results although objective loudness differed depending on the rendering conditions.

30

Rec. ITU-R  BS.1770-5

FIGURE 19

Results for five rendering algorithms

References

[1]

[2]

[3]

Sizle  A.,  Sporer  T.,  Liebetrau  J.,  and  Oode  S.  (2015),  “Progress  in  Standardization  of  3D-Audio
Loudness  in ITU-R  BS.1770,”  Proceedings  of  the 3rd  International  Conference  on  Spatial Audio,
paper 013.

Norcross S., Nanda S., Cohen Z. (2016), “ITU-R BS.1770 Based Loudness for Immersive Audio”,
the 140th Convention of the Audio Engineering Society, Convention Paper 9500.

Iwasaki T., Kubo H., and Oode S. (2022), “Loudness of Next-Generation Audio contents depending
on  rendering  conditions,”  the  154th  Convention  of  the  Audio  Engineering  Society,  Convention
Express Paper 77.


