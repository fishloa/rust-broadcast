## Table 3 — Application control code values
_§5.2.4.1, PDF pp. 19-19_

<!-- Auto-transcription truncated after 0x05; rows 0x06–0xFF hand-completed verbatim from the PDF (2026-06-12). -->

| MPEG-2 encoding | Identifier | Semantics |
|---|---|---|
| 0x00 |  | reserved_future_use |
| 0x01 | AUTOSTART | The application shall be started when the service is selected, unless the application is already running. |
| 0x02 | PRESENT | The application is allowed to run while the service is selected, however it shall not start automatically when the service becomes selected. |
| 0x03 | DESTROY | The application shall be stopped but may be permitted the opportunity to close down gracefully. Attempts to start the application shall fail. |
| 0x04 | KILL | The application shall be stopped as soon as possible. Attempts to start the application shall fail. |
| 0x05 | PREFETCH | Application files should be cached by the receiver, if possible. The application shall not be started and attempts to start it shall fail. |
| 0x06 | REMOTE | This identifies an application that is not available on the current transport stream and hence only available after tuning to a new transport stream or if cached and signalled as launchable completely from cache. |
| 0x07 | DISABLED | The application shall not be started and attempts to start it shall fail. |
| 0x08 | PLAYBACK_AUTOSTART | The application shall not be run, neither direct from broadcast nor when in timeshift mode. When a recording is being played back from storage, the application shall be presented as if it was autostart. |
| 0x09 to 0xFF |  | reserved_future_use |

