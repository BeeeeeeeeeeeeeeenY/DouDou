# DouDou

DouDou is a personal demo for an eye-friendly child learning companion:
phone for listening and speaking, a server for memory and teaching strategy,
and reMarkable Paper Pro for calm handwriting, drawing, and paper-like display.

The current prototype starts from a modified version of
[MaximeRivest/riddle](https://github.com/MaximeRivest/riddle). Riddle proves
the important device-side idea: capture ink, send a page image to an AI model,
and draw the reply back onto e-paper. DouDou will move the product brain to a
server so prompts, knowledge, memory, voice, and multi-device output can be
configured without rebuilding the tablet app.

## Current State

- `device/riddle`: modified reMarkable Paper Pro app prototype.
- `device/remarkable`: Xovi/AppLoad launcher notes and device-side entry files.
- `tools/mac`: local helper commands for configuring and controlling the
  connected device.
- `docs`: architecture and roadmap notes for the three-part product.

The public repository intentionally does not include API keys, `oracle.env`, SDK
folders, build outputs, or third-party font binaries whose redistribution rights
have not been confirmed.

## Direction

DouDou's intended architecture is:

```text
Phone
  speech input + audio playback
        |
Server
  STT, knowledge retrieval, prompt policies, long-term memory, TTS, layout
        |
reMarkable Paper Pro
  handwriting, drawing, e-paper cards, quiet visual feedback
```

The next milestone is a small server that exposes one endpoint, receives either
text or an image turn, applies a configurable child-friendly teaching profile,
and returns structured output for both the phone and the tablet.

## Local Notes

The device prototype currently expects `device/riddle/fonts/PingFangShiGuang.ttf`
to exist locally when building. Keep that file outside Git until the font license
is confirmed for redistribution.

