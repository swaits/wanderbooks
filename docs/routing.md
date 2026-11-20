# Routing: which model gets which delegated work

The orchestrating session plans, integrates and reviews. Bounded pieces go
to other models through `delegate_task`. This table is the current plan;
where it differs from the starting tiers the owner set on 2026-11-09, this
file wins. The evidence is `docs/delegation-log.md`, one line per task.

| Task type | Provider | Model | Effort |
|---|---|---|---|
| scout (find, summarize, rename, boilerplate, small edits) | `claudeAgent` | `claude-haiku-5-5` | effort medium |
| run/verify (run suites, reproduce, collect logs, check a build) | `muse` | `muse-ember-1.0` | reasoningEffort high |
| implement, refactor (well specified, clear acceptance) | `opencode` | `opencode-go/deepseek-v4.1-flash` | variant high |
| debug, hard implement (several modules, subtle, or tier 2 failed once) | `claudeAgent` | `claude-sonnet-5-5` | effort high |
| review (a risky design, or a large change before merge) | `muse` | `muse-wonder-full-1.0` | reasoningEffort high |

Note on run/verify: Muse Ember was released on 2026-11-09 and the owner's
rule was to use it from the day the Muse Code provider listed it, not
through OpenCode. `orchestrator_capabilities` listed `muse` /
`muse-ember-1.0` (option `reasoningEffort`, default high) on 2026-11-20.
Its first task here (2026-11-20) failed at once with "model does not
exist or you lack access", as a try on 2026-11-16 had: the provider lists
a model the account cannot use. Until a task runs on it, run-only tasks
go to the scout row; try it again at the next batch.

Rules that do not change with the data: set provider, model and effort on
every call; two tiers fit, take the cheaper; two failures, move up a tier
or do it in the orchestrating session; never raise Sonnet to max; no volume
work to the review tier without asking.

## Changes

- 2026-11-09: the starting tiers, as the owner gave them. No task type has
  5 logged samples yet, so nothing has moved.
- 2026-11-20: run/verify moves to Muse Ember through Muse Code, which now
  lists it. No sample yet.
