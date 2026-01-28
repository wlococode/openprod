# Contributing

This project is in early design. The most valuable contributions right now are **ideas, feedback, and discussion**—not code.

---

## How You Can Help

### Review the Architecture

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and tell me:

- What's unclear or confusing?
- What assumptions seem wrong?
- Where will this break in practice?
- What's missing?

Open an issue with the **architecture** label.

### Challenge the Sync Model

If you have experience with distributed systems, CRDTs, or offline-first architectures:

- Where will the peer-to-peer replication fail?
- Is leader election the right approach?
- What edge cases am I missing?
- Are there existing solutions I should study?

Open an issue with the **sync** label.

### Weigh In on Concepts & Bindings

The cross-plugin identity system is my biggest uncertainty. If you've worked with:

- RDF / linked data
- Notion relations
- Airtable linked records
- Contact merging systems
- Entity resolution in any domain

I'd love your perspective. Open an issue with the **identity** label.

### Share Production Workflows

If you work in entertainment production:

- What tools do you use today?
- What's broken about team collaboration?
- Which of the example scenarios in the architecture doc would actually help you?
- What's missing?

Open an issue with the **workflows** label.

### Suggest Plugins

What would you build on top of this system?

- What data would it manage?
- How would it interact with other plugins?
- What capabilities would it need?

Open an issue with the **plugins** label.

---

## What I'm Looking For in Collaborators

I don't expect anyone to build this for me. But if you're interested in being involved long-term:

- **Distributed systems folks** — Help get the sync and conflict model right
- **Rust developers** — Eventually, help build a clean implementation of the core
- **Production professionals** — Keep the design grounded in real workflows
- **Skeptics** — Tell me where this will fall apart

---

## Code Contributions

Not yet. When the architecture is solid and we're ready to build, this section will include:

- Development setup
- Code style guidelines
- PR process
- Testing requirements

---

## Discussion Guidelines

- Be specific. "This seems wrong" is less useful than "This will break when X because Y."
- Be constructive. Criticism is valuable; hostility is not.
- Assume good intent.

---

## Questions?

Open an issue with the **question** label, start a discussion, or email openprod@wloco.me
