# RTSP Protocol State Machines

**Source:** RFC 2326 Appendix A (§A.1, §A.2)

State is defined on a per-object basis. An object is uniquely identified by the
stream URL and the RTSP session identifier. Requests using aggregate URLs affect
the individual states of all constituent streams.

The requests **OPTIONS, ANNOUNCE, DESCRIBE, GET_PARAMETER, and SET_PARAMETER** do
not affect client or server state and are therefore not listed in the state tables.
(RFC 2326 Appendix A.)

---

## A.1 Client State Machine

**Cite:** RFC 2326 Appendix A.1

### States

**Init:**
SETUP has been sent, waiting for reply.

**Ready:**
SETUP reply received or PAUSE reply received while in Playing state.

**Playing:**
PLAY reply received.

**Recording:**
RECORD reply received.

### Transition rules

The client changes state on receipt of replies to requests. The "next state"
column indicates the state assumed after receiving a **success response (2xx)**.

- A **3xx** response causes the state to become **Init**.
- A **4xx** response causes **no change** in state.

Messages not listed for each state **MUST NOT** be issued by the client in that
state (except messages that do not affect state, as listed above). Receiving a
REDIRECT from the server is equivalent to receiving a 3xx redirect status.

If no explicit SETUP is required for the object (e.g., it is available via a
multicast group), state begins at **Ready**. In this case there are only two
states: Ready and Playing. The client also changes state from Playing/Recording
to Ready when the end of the requested range is reached.

### Client state table

```
state       message sent     next state after response
Init        SETUP            Ready
            TEARDOWN         Init
Ready       PLAY             Playing
            RECORD           Recording
            TEARDOWN         Init
            SETUP            Ready
Playing     PAUSE            Ready
            TEARDOWN         Init
            PLAY             Playing
            SETUP            Playing (changed transport)
Recording   PAUSE            Ready
            TEARDOWN         Init
            RECORD           Recording
            SETUP            Recording (changed transport)
```

---

## A.2 Server State Machine

**Cite:** RFC 2326 Appendix A.2

### States

**Init:**
The initial state; no valid SETUP has been received yet.

**Ready:**
Last SETUP received was successful and reply sent, or after playing, last PAUSE
received was successful and reply sent.

**Playing:**
Last PLAY received was successful and reply sent. Data is being sent.

**Recording:**
The server is recording media data.

### Transition rules

The server changes state on receiving requests. The "next state" column indicates
the state assumed after **sending a success response (2xx)**.

- A **3xx** response causes the state to become **Init**.
- A **4xx** response causes **no change** in state.

**Idle timeouts:**

- If the server is in state **Playing** or **Recording** (unicast) and has not
  received "wellness" information (RTCP reports or RTSP commands) from the client
  for a defined interval, the server **MAY** revert to **Init** and tear down the
  RTSP session. The default interval is **60 seconds** (1 minute). The server can
  declare a different timeout value in the `Session` response header (§12.37).
- If the server is in state **Ready**, it **MAY** revert to **Init** if it does
  not receive an RTSP request for more than **60 seconds**.

The server reverts from Playing or Recording to Ready at the end of the range
requested by the client.

If no explicit SETUP is required for the object, state starts at **Ready** and
there are only two states: Ready and Playing.

**Engine behaviour for unlisted messages:** messages not listed for a given state
MUST NOT be issued by the client in that state; if the server receives such a
message it MUST return **455 Method Not Valid In This State** (§11.3.6).

### Server state table

```
state           message received  next state
Init            SETUP             Ready
                TEARDOWN          Init
Ready           PLAY              Playing
                SETUP             Ready
                TEARDOWN          Init
                RECORD            Recording
Playing         PLAY              Playing
                PAUSE             Ready
                TEARDOWN          Init
                SETUP             Playing
Recording       RECORD            Recording
                PAUSE             Ready
                TEARDOWN          Init
                SETUP             Recording
```
