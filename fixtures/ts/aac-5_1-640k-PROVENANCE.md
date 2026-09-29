# `aac-5_1-640k.ts`

AAC 5.1 at 640 kb/s in MPEG-TS, generated locally with ffmpeg 8.1:

```
ffmpeg -y -f lavfi -i "anoisesrc=r=48000:a=1:d=3" \
  -af "pan=5.1|FL=FL|FR=FR|FC=FC|LFE=LFE|BL=BL|BR=BR" \
  -c:a aac -b:a 640k -f mpegts aac-5_1-640k.ts
```

Committed because this is exactly the case RFC 3640 §3.2.3.1's access-unit
fragmentation exists for: a ~3.7 kB AAC frame is far larger than a typical RTP
payload budget, so a real sender splits it over several packets. The workspace's
other AAC fixtures are stereo and low-rate, whose access units fit one packet,
so they cannot exercise the fragmented path at all.
