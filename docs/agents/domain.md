# Domain docs

## Layout and reading rules

This repository uses a single domain context:

- `GLOSSARY.md` at the repository root defines shared terminology.
- `docs/adr/` holds architectural decision records.

Before exploring domain concepts, read the glossary and ADRs relevant
to the area being changed.

If these files do not exist, proceed silently. The domain-modeling
skill creates them when terms or decisions are resolved.

## Vocabulary

Use glossary terms in issue titles, proposals, hypotheses, and tests.
If a needed concept is missing, reconsider the terminology or note
the gap for domain-modeling.

## Decision conflicts

Explicitly identify any proposal that contradicts an existing ADR,
reference that ADR, and explain why the decision should be reconsidered.
