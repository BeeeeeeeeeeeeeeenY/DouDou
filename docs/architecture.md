# Architecture

## Product Shape

DouDou should not put the whole AI product inside the tablet. The tablet is the
paper surface: excellent for eye comfort, handwriting, drawing, and calm visual
feedback. The phone handles microphone and speaker work. The server owns the
product intelligence.

## Components

### Phone

- Captures parent or child speech.
- Plays the teaching voice.
- Shows a lightweight companion view for setup and debugging.
- Sends turns to the same server used by the tablet.

### Server

- Stores child profile, family preferences, teaching style, and memories.
- Retrieves from a curated knowledge base before generation.
- Applies prompt policies such as language, age level, answer length, and tone.
- Returns structured output instead of one free-form answer.
- Generates or selects audio for phone playback.
- Produces tablet-friendly visual instructions.

### reMarkable Paper Pro

- Captures pen strokes and drawings.
- Sends page images or stroke data to the server.
- Renders short text, simple cards, and line-art responses.
- Avoids owning knowledge, memory, prompt policy, or speech.

## Server Response Shape

The server should eventually return a structure like:

```json
{
  "spoken_text": "这是小星星。我们一起数一数有几颗。",
  "paper_text": "小星星：1、2、3",
  "paper_cards": [
    { "type": "text", "content": "数一数星星" },
    { "type": "sketch", "prompt": "三颗简单星星，适合电子纸线稿" }
  ],
  "memory_tags": ["counting", "stars", "drawing"]
}
```

This keeps the phone, server, and tablet aligned without asking the model to
invent UI behavior in free text.

