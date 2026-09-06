# SPDX-License-Identifier: MIT
# Per-replica journal schema (`journal.jsonl`, one JSON object per line).
#
# Written by the app after every pasted replica to
# `%APPDATA%\WispFlowStealer\journal.jsonl` (T-01). Corrupt lines are
# skipped on read; the file is append-only (history lives separately in
# `history.json`, so clearing statistics never loses replicas).

| field | type | meaning |
|---|---|---|
| `id` | string | unique id: `<unix_nanos>-<process_counter>` (T-13) |
| `ts` | integer | Unix seconds, UTC (T-12; consumers render with timezone) |
| `backend` | string | engine used: `groq cloud` / `whisper local` / `vosk` |
| `lang` | string | `ru` / `en` / `auto` |
| `app` | string | foreground window title at release time (may be empty) |
| `chars` | integer | final text length in chars |
| `words` | integer | word count of the final text |
| `secs` | number | seconds from hotkey release to finished paste |
| `wpm` | number | words per minute over the captured audio span (AL-02; 0 when unknown) |
| `audio_secs` | number | captured audio span in seconds |
| `method` | string | delivery method: `paste` |

Example line:

```json
{"id":"1725580200123456789-3","ts":1725580200,"backend":"groq cloud","lang":"ru","app":"Telegram","chars":24,"words":4,"secs":2.31,"wpm":96.0,"audio_secs":2.5,"method":"paste"}
```

Aggregates (`--stats`, GUI statistics): replicas/words per day and total,
average `secs`, average/best `wpm` (averaged over measurable replicas).
CSV export (`stats.csv`) carries the same columns in order.
