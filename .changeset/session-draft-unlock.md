---
"@treramey/graduate": patch
---

Release a preserved restack session's lock as soon as the draft is dropped so the session can be resumed in the same process; fixes an intermittent `session_locked` failure.
