## Table 113 — default icon signalling
_§11.1, PDF pp. 99-99_

| default_icon_flag | icon_id | Description |
|---|---|---|
| '0' | 0x0 | Do not render anything e.g. an icon is already rendered in the video. |
| '0' | 0x1 to 0x7 | Render icon signalled by descriptor (and only this). |
| '1' | 0x0 | Render receiver default icon. |
| '1' | 0x1 to 0x7 | Render icon signalled by descriptor by preference but if this is not yet cached (URL only mechanism) use receiver default icon. |

