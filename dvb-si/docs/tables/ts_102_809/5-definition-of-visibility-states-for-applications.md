## Table 5 — Definition of visibility states for applications
_§5.2.6.1, PDF pp. 21-21_

| MPEG-2 encoding | XML Encoding | Description |
|---|---|---|
| 00 | NOT_VISIBLE_ALL | This application shall not be visible either to applications via an application listing API (if such an API is supported by the receiver) or to users via the navigator with the exception of any error reporting or logging facility, etc. |
| 01 | NOT_VISIBLE_USERS | This application shall not be visible to users but shall be visible to applications via an application listing API (if such an API is supported by the receiver). |
| 10 | | reserved_future_use |
| 11 | VISIBLE_ALL | This application can be visible to users and shall be visible to applications via an application listing API (if such an API is supported by the receiver). |

