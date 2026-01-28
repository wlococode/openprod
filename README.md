# openprod

An offline-first collaboration system for entertainment production teams.

---

## What is this?

Production teams juggle too many tools. Lighting has Vectorworks and Lightwright. Stage management has Word and Excel. Sound has their apps. Everyone digs through Slack and their email trying to find what's "current."

The daily reality: stage managers calculate call times by hand for 50 people, lighting designers email Excel exports every night, technical directors build from outdated drafts. When something changes, every related document needs manual updates. Something always gets missed.

**openprod** is different:

- **Offline-first** — Works without internet. Your data lives on your machine.
- **LAN-collaborative** — Sync directly with your team over local network. No cloud subscription required.
- **Plugin-extensible** — Core handles storage and sync. Plugins provide workflows.
- **Conflict-aware** — When offline edits collide, you see what happened and decide the resolution.

The core insight: most production paperwork is _derived_ from the same underlying information. If the data lives in one place and relationships are explicit, everything can stay in sync.

---

## Project Status

**This project is in early design.** There's a rough prototype that proves the core ideas work, but it's not production-ready.

I'm a lighting and video designer, not a software engineer. I understand the problem deeply but some of the harder technical challenges are beyond my current skills. I've been working on this for about a year, and I keep hitting walls when the code gets complex. I want to solve this problem for people without relying on AI slop to do the work for me. There are so many capable and intelligent people who work with problems like these every day.

This isn't "please build my idea for free." I want to be actively involved, and I'll maintain what I can. But I need collaborators who can help with the parts I can't do alone.

---

## What's Needed

- **Architecture review** — Is the design sound? What's wrong? What am I missing?
- **Distributed systems expertise** — The peer-to-peer sync model needs scrutiny
- **Production domain knowledge** — What workflows actually matter? What would you use?
- **Eventually: implementation** — A clean Rust core, plugin runtime, basic UI

Read the [Architecture Overview](docs/ARCHITECTURE.md) for the full design. It's honest about what's firm vs. what I don't know.

---

## Why Open Source?

I want this to exist and be free for production teams to use. No subscriptions for basic functionality. No cloud dependency for people who just want to work locally. Powerful tools should be available and accessible to anyone who wants to create.

If it makes money someday, it'll be from optional cloud hosting or donations—not from locking core features behind a paywall.

---

## License

TBD. I want to prevent someone from taking this code and selling it as a proprietary product. Likely AGPL or similar copyleft license, but could do MIT or GPL. Open to input.

---

## Get Involved

- Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Open an issue with questions, critiques, or ideas
- See [CONTRIBUTING.md](CONTRIBUTING.md) for how to participate

If you work in production and this resonates—or if you're a systems person who thinks this is interesting—I'd like to hear from you.

You can contact me directly at openprod@wloco.me
