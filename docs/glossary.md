# Glossary

The words SolidDraft and Catallax use for planned work. [ROADMAP.md](../ROADMAP.md) uses them,
and so should issues, pull requests and docs.

## Units of work

There are three levels of work, and nothing else.

| Word | Meaning | Example |
|---|---|---|
| **Phase** | A block of work that ends in one release of Catallax and SolidDraft together, after a checkpoint. | Phase 5 |
| **Step** | A numbered goal inside a phase, with a "done when" line. It is measured at its end (speed runs, census, counts). Steps in one phase can run at the same time. | 5.1 profile everything |
| **Task** | One worker, one branch, one pull request. A **lane** is a worker doing a task. | "cache per face" |

## Other words

- **Checkpoint:** the measured report plus the owner's review at the end of a phase, before
  its release.
- **Coverage line:** one of the 61 abilities compared with OpenCASCADE and SMLib. It is a
  score, not a unit of work.

## Retired words

Do not use "stage", "item N", "job" or "milestone" for roadmap work. The old 66-item order,
its stages and its milestones map into the phases and steps of the roadmap. G numbers stay
only as references to the gap board. These words keep their ordinary meaning elsewhere: a
CI job, a kernel worker job and a USD stage are not roadmap work.
