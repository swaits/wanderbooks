# Routing: which model gets which delegated work

The orchestrating session plans, integrates and reviews. Bounded pieces go
to other models through `delegate_task`. This table is the current plan;
where it differs from the starting tiers the owner set on 2026-11-09, this
file wins. The evidence is `docs/delegation-log.md`, one line per task.

| Task type | Provider | Model | Effort |
|---|---|---|---|
| scout (find, summarize, rename, boilerplate, small edits) | `claudeAgent` | `claude-haiku-5-5` | effort medium |
| run/verify (run suites, reproduce, collect logs, check a build) | see the note | Muse Ember | |
| implement, refactor (well specified, clear acceptance) | `opencode` | `opencode-go/deepseek-v4.1-flash` | variant high |
| debug, hard implement (several modules, subtle, or tier 2 failed once) | `claudeAgent` | `claude-sonnet-5-5` | effort high |
| review (a risky design, or a large change before merge) | `muse` | `muse-wonder-full-1.0` | reasoningEffort high |

Note on run/verify: the plan names Muse Ember through Muse Code. Ember was
released on 2026-11-09 and the Muse Code provider did not list it yet (the
only listing was `opencode` / `opencode/muse-ember-1.0`). The owner's rule
(2026-11-09): keep checking Muse Code for it, and use it from the day it is
listed there, not through OpenCode. Check with `orchestrator_capabilities`
(`search: "ember"`) before each batch of delegations; until it is listed,
run-only tasks go to the scout row. Last checked 2026-11-09 21:05 MST: not
listed.

Rules that do not change with the data: set provider, model and effort on
every call; two tiers fit, take the cheaper; two failures, move up a tier
or do it in the orchestrating session; never raise Sonnet to max; no volume
work to the review tier without asking.

## Changes

- 2026-11-09: the starting tiers, as the owner gave them. No task type has
  5 logged samples yet, so nothing has moved.
